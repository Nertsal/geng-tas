use geng::prelude::*;

/// A wrapper for a game that implements TAS functionality:
/// save states, slow motion, input replay.
pub struct Tas<T: Tasable> {
    geng: Geng,
    /// The game state that is manipulated.
    game: T,
    show_ui: bool,
    /// Multiplier for `delta_time`, used for slow-motion.
    time_scale: f64,
    paused: bool,
    /// The expected time between fixed updates.
    fixed_delta_time: f64,
    /// The time until the next fixed update (if queued).
    next_fixed_update: Option<f64>,
    /// All saved states.
    saved_states: Vec<SaveState<T::Saved>>,
    /// Current simulation (not real) time.
    time: f64,
    /// History of all inputs.
    inputs: Vec<TickInput>,
    save_file: String,
    replay: Option<Replay>,
    initial_state: T::Saved,
    acc_delta_time: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SavedTas<S> {
    initial_state: S,
    inputs: Vec<TickInput>,
}

struct Replay {
    time: f64,
    inputs: Vec<TickInput>,
    next_input: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TickInput {
    time: f64,
    event: geng::Event,
}

#[derive(Clone, Serialize, Deserialize)]
struct SaveState<S> {
    time: f64,
    inputs: Vec<TickInput>,
    state: S,
}

/// Holds the implementation details of the game to be TAS'ed.
pub trait Tasable {
    /// A type used for saving and restoring the state of the game.
    type Saved: Clone + Serialize + serde::de::DeserializeOwned;

    /// Save current state.
    fn save(&self) -> Self::Saved;

    /// Restore a previously saved state.
    fn load(&mut self, state: Self::Saved);
}

impl<T: Tasable> Tas<T> {
    pub fn new(game: T, geng: &Geng) -> Self {
        let mut tas = Self {
            geng: geng.clone(),
            show_ui: true,
            time_scale: 1.0,
            paused: true,
            fixed_delta_time: 1.0,
            next_fixed_update: None,
            saved_states: Vec::new(),
            time: 0.0,
            inputs: Vec::new(),
            save_file: "tas.json".to_string(),
            replay: None,
            initial_state: game.save(),
            game,
            acc_delta_time: 0.0,
        };
        tas.load_savestates().expect("Failed to load saved states");
        tas
    }

    /// Saves the current game state.
    fn save_state(&mut self) {
        self.saved_states.push(SaveState {
            time: self.time,
            inputs: self.inputs.clone(),
            state: self.game.save(),
        });
        if let Err(err) = self.save_savestates() {
            error!("Failed to save states: {err}");
        }
    }

    /// Attempts to load the saved state by index.
    /// If such a state is not found, nothing happens.
    fn load_state(&mut self, index: usize) {
        // Get the state by index
        if let Some(state) = self.saved_states.get(index) {
            let state = state.clone();
            self.time = state.time;
            self.inputs = state.inputs;
            self.game.load(state.state);
        }
    }

    /// Saves the run in a file.
    fn save_run(
        &self,
        path: impl AsRef<std::path::Path>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let file = std::fs::File::create(path)?;
        let writer = std::io::BufWriter::new(file);
        let saved = SavedTas {
            initial_state: self.initial_state.clone(),
            inputs: self.inputs.clone(),
        };
        serde_json::to_writer_pretty(writer, &saved)?;
        Ok(())
    }

    /// Loads the run from the file.
    fn load_run(
        &mut self,
        path: impl AsRef<std::path::Path>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let file = std::fs::File::open(path)?;
        let reader = std::io::BufReader::new(file);
        let saved: SavedTas<T::Saved> = serde_json::from_reader(reader)?;
        self.game.load(saved.initial_state);
        self.replay = Some(Replay {
            time: 0.0,
            inputs: saved.inputs,
            next_input: 0,
        });
        Ok(())
    }

    fn save_savestates(&self) -> Result<(), Box<dyn std::error::Error>> {
        let file = std::fs::File::create("savedstates.json")?;
        let writer = std::io::BufWriter::new(file);
        serde_json::to_writer_pretty(writer, &self.saved_states)?;
        Ok(())
    }

    fn load_savestates(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let file = std::fs::File::open("savedstates.json")?;
        let reader = std::io::BufReader::new(file);
        self.saved_states = serde_json::from_reader(reader)?;
        Ok(())
    }
}

impl<T: geng::State + Tasable> geng::State for Tas<T> {
    fn draw(&mut self, framebuffer: &mut ugli::Framebuffer) {
        self.game.draw(framebuffer);
    }

    fn update(&mut self, _delta_time: f64) {}

    fn fixed_update(&mut self, delta_time: f64) {
        self.fixed_delta_time = delta_time;
        if self.next_fixed_update.is_none() {
            self.next_fixed_update = Some(delta_time);
        }

        if !self.paused {
            let mut play_time = self.acc_delta_time + delta_time * self.time_scale;
            while play_time >= delta_time {
                if let Some(replay) = &mut self.replay {
                    replay.time += delta_time;
                    while replay.next_input < replay.inputs.len()
                        && replay.inputs[replay.next_input].time <= replay.time
                    {
                        self.game
                            .handle_event(replay.inputs[replay.next_input].event.clone());
                        replay.next_input += 1;
                    }
                }

                self.time += delta_time;
                self.game.update(delta_time);

                if let Some(time) = &mut self.next_fixed_update {
                    // Simulate fixed updates manually
                    *time -= delta_time;
                    let mut updates = 0;
                    while *time <= 0.0 {
                        *time += self.fixed_delta_time;
                        updates += 1;
                    }
                    for _ in 0..updates {
                        self.game.fixed_update(self.fixed_delta_time);
                    }
                }

                play_time -= delta_time;
            }
            self.acc_delta_time = play_time;
        }
    }

    fn handle_event(&mut self, event: geng::Event) {
        let window = self.geng.window();
        if window.is_key_pressed(geng::Key::LAlt) {
            // Capture the event
            if let geng::Event::KeyDown { key } = event {
                match key {
                    geng::Key::S => {
                        self.save_run("tas.json").unwrap();
                    }
                    geng::Key::P => {
                        self.paused = !self.paused;
                    }
                    _ => {}
                }
            }
            return;
        }

        if self.replay.is_some() {
            return;
        }

        self.inputs.push(TickInput {
            time: self.time,
            event: event.clone(),
        });
        self.game.handle_event(event);
    }

    fn ui<'a>(&'a mut self, ui: &'a geng::ui::Controller) -> Box<dyn geng::ui::Widget + 'a> {
        if !self.show_ui {
            return self.game.ui(ui);
        }

        use geng::ui::{column, *};
        let mut load_state = None;
        let mut delete_state = None;
        let mut saved_states: Vec<_> = self
            .saved_states
            .iter()
            .enumerate()
            .map(|(i, _)| {
                Box::new(
                    row![
                        Text::new(
                            format!("Save #{i}"),
                            self.geng.default_font().clone(),
                            30.0,
                            Rgba::BLACK
                        ),
                        {
                            let load_save = Button::new(ui, "Load");
                            if load_save.was_clicked() {
                                load_state = Some(i);
                            }
                            load_save
                        }
                        .padding_horizontal(20.0),
                        {
                            let delete_save = Button::new(ui, "Delete");
                            if delete_save.was_clicked() {
                                delete_state = Some(i);
                            }
                            delete_save
                        }
                        .padding_horizontal(20.0),
                    ]
                    .padding_vertical(10.0),
                ) as Box<dyn Widget>
            })
            .collect();
        if let Some(i) = delete_state {
            self.saved_states.remove(i);
        } else if let Some(i) = load_state {
            self.load_state(i);
        }

        let tas_ui = stack![
            if self.paused {
                Box::new(
                    Text::new(
                        "Paused",
                        self.geng.default_font().clone(),
                        50.0,
                        Rgba::BLACK,
                    )
                    .align(vec2(1.0, 0.9)),
                ) as Box<dyn Widget>
            } else {
                Box::new(Void)
            },
            column![
                {
                    let time_slider = slider::Slider::new(ui, self.time_scale, 0.0..=10.0);
                    if let Some(value) = time_slider.get_change() {
                        self.time_scale = value;
                    }
                    time_slider
                },
                Text::new(
                    format!("Time scale: {:.2}", self.time_scale),
                    self.geng.default_font().clone(),
                    50.0,
                    Rgba::BLACK
                )
                .center()
            ]
            .align(vec2(0.5, 1.0)),
            column![
                Text::new(
                    self.save_file.clone(),
                    self.geng.default_font().clone(),
                    40.0,
                    Rgba::BLACK
                ),
                row![
                    {
                        let save_run = Button::new(ui, "Save run");
                        if save_run.was_clicked() {
                            if let Err(err) = self.save_run(&self.save_file) {
                                error!("Failed to save run: {err}");
                            }
                        }
                        save_run
                    },
                    {
                        let load_run = Button::new(ui, "Start replay");
                        if load_run.was_clicked() {
                            println!("The evil button");
                            if let Err(err) = self.load_run(&self.save_file.clone()) {
                                error!("Failed to load run: {err}");
                            }
                        }
                        load_run
                    }
                ]
            ]
            .align(vec2(0.0, 0.0)),
            column({
                saved_states.push(Box::new({
                    let new_save = Button::new(ui, "Save state");
                    if new_save.was_clicked() {
                        self.save_state();
                    }
                    new_save
                }) as Box<dyn Widget>);
                saved_states
            })
            .align(vec2(1.0, 0.0))
            .padding_bottom(200.0)
        ]
        .uniform_padding(30.0);

        Box::new(stack(vec![self.game.ui(ui), Box::new(tas_ui)]))
    }
}

mod slider {
    use super::*;
    use geng::ui::*;

    pub struct Slider<'a> {
        cx: &'a Controller,
        sense: &'a mut Sense,
        pos: &'a mut Option<AABB<f64>>,
        tick_radius: &'a mut f32,
        value: f64,
        range: RangeInclusive<f64>,
        change: RefCell<&'a mut Option<f64>>,
    }

    impl<'a> Slider<'a> {
        const ANIMATION_SPEED: f32 = 5.0;

        pub fn new(cx: &'a Controller, value: f64, range: RangeInclusive<f64>) -> Self {
            Slider {
                cx,
                sense: cx.get_state(),
                tick_radius: cx.get_state(),
                pos: cx.get_state(),
                value,
                range,
                change: RefCell::new(cx.get_state()),
            }
        }

        pub fn get_change(&self) -> Option<f64> {
            self.change.borrow_mut().take()
        }
    }

    impl<'a> Widget for Slider<'a> {
        fn sense(&mut self) -> Option<&mut Sense> {
            Some(self.sense)
        }
        fn update(&mut self, delta_time: f64) {
            let target_tick_radius = if self.sense.is_hovered() || self.sense.is_captured() {
                1.0 / 2.0
            } else {
                1.0 / 6.0
            };
            *self.tick_radius += (target_tick_radius - *self.tick_radius)
                .clamp_abs(Self::ANIMATION_SPEED * delta_time as f32);
        }
        fn draw(&mut self, cx: &mut DrawContext) {
            *self.pos = Some(cx.position);
            let geng = cx.geng;
            let draw_2d = geng.draw_2d_helper();
            let position = cx.position.map(|x| x as f32);
            let line_width = position.height() / 3.0;
            let value_position = if self.range.end() == self.range.start() {
                *self.tick_radius
            } else {
                *self.tick_radius
                    + ((self.value - *self.range.start())
                        / (*self.range.end() - *self.range.start())) as f32
                        * (position.width() - line_width)
            };
            geng.draw_2d(
                cx.framebuffer,
                &geng::PixelPerfectCamera,
                &draw_2d::Quad::new(
                    AABB::from_corners(
                        position.bottom_left()
                            + vec2(value_position, (position.height() - line_width) / 2.0),
                        position.top_right()
                            - vec2(line_width / 2.0, (position.height() - line_width) / 2.0),
                    ),
                    cx.theme.usable_color,
                ),
            );
            draw_2d.circle(
                cx.framebuffer,
                &geng::PixelPerfectCamera,
                position.top_right() - vec2(line_width / 2.0, position.height() / 2.0),
                line_width / 2.0,
                cx.theme.usable_color,
            );
            geng.draw_2d(
                cx.framebuffer,
                &geng::PixelPerfectCamera,
                &draw_2d::Quad::new(
                    AABB::from_corners(
                        position.bottom_left()
                            + vec2(line_width / 2.0, (position.height() - line_width) / 2.0),
                        position.bottom_left()
                            + vec2(value_position, (position.height() + line_width) / 2.0),
                    ),
                    cx.theme.hover_color,
                ),
            );
            geng.draw_2d(
                cx.framebuffer,
                &geng::PixelPerfectCamera,
                &draw_2d::Ellipse::circle(
                    position.bottom_left() + vec2(line_width / 2.0, position.height() / 2.0),
                    line_width / 2.0,
                    cx.theme.hover_color,
                ),
            );
            draw_2d.circle(
                cx.framebuffer,
                &geng::PixelPerfectCamera,
                position.bottom_left() + vec2(value_position, position.height() / 2.0),
                *self.tick_radius * position.height(),
                cx.theme.hover_color,
            );
        }
        fn handle_event(&mut self, event: &geng::Event) {
            let aabb = match *self.pos {
                Some(pos) => pos,
                None => return,
            };
            if self.sense.is_captured() {
                if let geng::Event::MouseDown { position, .. }
                | geng::Event::MouseMove { position, .. } = &event
                {
                    let position = position.x - aabb.x_min;
                    let new_value = *self.range.start()
                        + ((position - aabb.height() / 6.0) / (aabb.width() - aabb.height() / 3.0))
                            .clamp(0.0, 1.0)
                            * (*self.range.end() - *self.range.start());
                    **self.change.borrow_mut() = Some(new_value);
                }
            }
        }

        fn calc_constraints(&mut self, _children: &ConstraintsContext) -> Constraints {
            Constraints {
                min_size: vec2(1.0, 1.0) * self.cx.theme().text_size as f64,
                flex: vec2(1.0, 0.0),
            }
        }
    }
}
