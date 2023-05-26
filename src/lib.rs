use geng::prelude::*;

mod ui;

/// A wrapper for a game that implements TAS functionality:
/// save states, slow motion, input replay.
pub struct Tas<T: Tasable> {
    geng: Geng,
    framebuffer_size: vec2<usize>,
    /// The game state that is manipulated.
    game: T,
    show_ui: bool,
    /// Multiplier for `delta_time`, used for slow-motion.
    time_scale: f64,
    paused: bool,
    /// Paused when pressing the LAlt key.
    auto_paused: bool,
    /// The expected time between fixed updates.
    fixed_delta_time: f64,
    /// All saved states.
    saved_states: Vec<SaveState<T::Saved>>,
    /// Current simulation frame.
    frame: usize,
    /// History of all inputs.
    inputs: Vec<FrameInput<geng::Event>>,
    save_file: String,
    replay: Option<Replay<geng::Event>>,
    initial_state: T::Saved,
    acc_delta_time: f64,
    queued_inputs: Vec<geng::Event>,
    /// All pressed keyboard keys in the simulation.
    pressed_keys: HashSet<geng::Key>,
    /// All pressed mouse buttons in the simulation.
    pressed_buttons: HashSet<geng::MouseButton>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SavedTas<T> {
    initial_state: T,
    inputs: Vec<FrameInput<geng::Event>>,
}

struct Replay<T> {
    /// Current frame index.
    frame: usize,
    /// Current input index.
    input: usize,
    /// The amount of frames until next input should be taken.
    next_input: usize,
    inputs: Vec<FrameInput<T>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FrameInput<T> {
    /// How long should these inputs be replayed for.
    frames: usize,
    inputs: Vec<T>,
}

#[derive(Clone, Serialize, Deserialize)]
struct SaveState<T> {
    frame: usize,
    inputs: Vec<FrameInput<geng::Event>>,
    pressed_keys: HashSet<geng::Key>,
    pressed_buttons: HashSet<geng::MouseButton>,
    initial_state: T,
    state: T,
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

impl<T: geng::State + Tasable> Tas<T> {
    pub fn new(game: T, geng: &Geng) -> Self {
        let mut tas = Self {
            geng: geng.clone(),
            framebuffer_size: vec2(1, 1),
            show_ui: true,
            time_scale: 1.0,
            paused: true,
            auto_paused: false,
            fixed_delta_time: 1.0,
            saved_states: Vec::new(),
            frame: 0,
            inputs: Vec::new(),
            save_file: "tas.json".to_string(),
            replay: None,
            initial_state: game.save(),
            game,
            acc_delta_time: 0.0,
            queued_inputs: Vec::new(),
            pressed_keys: HashSet::new(),
            pressed_buttons: HashSet::new(),
        };
        tas.load_savestates().expect("Failed to load saved states");
        tas
    }

    /// Saves the current game state.
    fn save_state(&mut self) {
        self.saved_states.push(SaveState {
            frame: self.frame,
            inputs: self.inputs.clone(),
            initial_state: self.initial_state.clone(),
            state: self.game.save(),
            pressed_keys: self.pressed_keys.clone(),
            pressed_buttons: self.pressed_buttons.clone(),
        });
        if let Err(err) = self.save_savestates() {
            log::error!("Failed to save states: {err}");
        }
    }

    /// Attempts to load the saved state by index.
    /// If such a state is not found, nothing happens.
    fn load_state(&mut self, index: usize) {
        // Stop replay
        self.replay.take();

        // Get the state by index
        if let Some(state) = self.saved_states.get(index) {
            let state = state.clone();
            self.frame = state.frame;
            self.inputs = state.inputs;
            self.pressed_keys = state.pressed_keys;
            self.pressed_buttons = state.pressed_buttons;
            self.initial_state = state.initial_state;
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
        let saved = SavedTas::<T::Saved> {
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
        self.frame = 0;
        self.queued_inputs.clear();
        self.inputs.clear();
        self.pressed_keys.clear();
        self.pressed_buttons.clear();
        self.replay = Some(Replay {
            frame: 0,
            input: 0,
            next_input: saved.inputs.first().map(|input| input.frames).unwrap_or(0),
            inputs: saved.inputs,
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
        let Ok(file) = std::fs::File::open("savedstates.json") else {
            log::warn!("Failed to open savedstates.json");
            self.saved_states = default();
            return Ok(());
        };
        let reader = std::io::BufReader::new(file);
        self.saved_states = serde_json::from_reader(reader)?;
        Ok(())
    }

    /// Plays the next frame (either in replay or record mode).
    fn next_frame(&mut self) {
        // Get frame inputs
        let inputs = if let Some(replay) = &self.replay {
            match replay.inputs.get(replay.input) {
                Some(inputs) => &inputs.inputs,
                None => {
                    // TODO: indicate that the replay has ended or smth
                    self.paused = true;
                    return;
                }
            }
        } else {
            &self.queued_inputs
        };

        // Simulate inputs
        for input in inputs {
            // Update pressed states
            match input {
                geng::Event::KeyDown { key } => {
                    self.pressed_keys.insert(*key);
                }
                geng::Event::KeyUp { key } => {
                    self.pressed_keys.remove(key);
                }
                geng::Event::MouseDown { button, .. } => {
                    self.pressed_buttons.insert(*button);
                }
                geng::Event::MouseUp { button, .. } => {
                    self.pressed_buttons.remove(button);
                }
                _ => {}
            }
            // Sync pressed states
            self.geng
                .window()
                .set_pressed_keys(self.pressed_keys.clone());
            self.geng
                .window()
                .set_pressed_buttons(self.pressed_buttons.clone());

            self.game.handle_event(input.clone());
        }

        // Update inputs
        if let Some(replay) = &mut self.replay {
            if replay.input < replay.inputs.len() {
                // Get next input
                replay.next_input = replay.next_input.saturating_sub(1);
                if replay.next_input == 0 {
                    replay.input += 1;
                    if let Some(next) = replay.inputs.get(replay.input) {
                        replay.next_input = next.frames;
                    }
                }

                replay.frame += 1;
            }
        } else {
            // Record the inputs
            let inputs = std::mem::take(&mut self.queued_inputs);
            if let Some(last) = self.inputs.last_mut().filter(|last| last.inputs == inputs) {
                // Extend last input
                last.frames += 1;
            } else {
                // Create new input
                self.inputs.push(FrameInput { frames: 1, inputs });
            }
        }

        // Update
        self.game.update(self.fixed_delta_time);
        self.game.fixed_update(self.fixed_delta_time);
        self.frame += 1;
    }
}

impl<T: geng::State + Tasable> geng::State for Tas<T> {
    fn draw(&mut self, framebuffer: &mut ugli::Framebuffer) {
        self.framebuffer_size = framebuffer.size();
        self.game.draw(framebuffer);
    }

    fn update(&mut self, _delta_time: f64) {}

    fn fixed_update(&mut self, delta_time: f64) {
        self.fixed_delta_time = delta_time;
        if !self.paused && !self.auto_paused {
            let mut sim_time = self.acc_delta_time + delta_time * self.time_scale;
            while sim_time >= self.fixed_delta_time {
                sim_time -= self.fixed_delta_time;
                self.next_frame();
            }
            self.acc_delta_time = sim_time;
        }
    }

    fn handle_event(&mut self, event: geng::Event) {
        if let geng::Event::KeyDown {
            key: geng::Key::LAlt,
        } = event
        {
            self.auto_paused = true;
        }
        if let geng::Event::KeyUp {
            key: geng::Key::LAlt,
        } = event
        {
            self.auto_paused = false;
        }

        if self.auto_paused {
            // Capture the event
            if let geng::Event::KeyDown { key } = event {
                match key {
                    geng::Key::S => {
                        self.save_run("tas.json").unwrap();
                    }
                    geng::Key::K => {
                        self.save_state();
                    }
                    geng::Key::L if !self.saved_states.is_empty() => {
                        self.load_state(self.saved_states.len() - 1);
                    }
                    geng::Key::P => {
                        self.paused = !self.paused;
                    }
                    geng::Key::Left => {
                        self.time_scale = (self.time_scale - 0.05).max(0.0);
                    }
                    geng::Key::Right => {
                        self.time_scale += 0.05;
                    }
                    _ => {}
                }
            }
            return;
        }

        if self.replay.is_some() {
            return;
        }

        self.queued_inputs.push(event);
    }

    fn ui<'a>(&'a mut self, cx: &'a geng::ui::Controller) -> Box<dyn geng::ui::Widget + 'a> {
        if !self.show_ui {
            return self.game.ui(cx);
        }

        use geng::ui::{column, *};

        let framebuffer_size = self.framebuffer_size.map(|x| x as f32);
        let text_size = framebuffer_size.y * 0.05;

        let font = self.geng.default_font().clone();
        let slider = move |name, range, value: &mut f64, text_size| {
            ui::slider(cx, name, value, range, font.clone(), text_size)
        };

        let font = self.geng.default_font().clone();
        let text =
            move |text, text_size| geng::ui::Text::new(text, font.clone(), text_size, Rgba::WHITE);

        macro_rules! button {
            ($name:expr => $callback:block) => {{
                let button = Button::new(cx, $name);
                if button.was_clicked() {
                    $callback
                }
                button
            }};
        }

        let mut load_state = None;
        let mut delete_state = None;
        let mut saved_states: Vec<_> = self
            .saved_states
            .iter()
            .enumerate()
            .map(|(i, _)| {
                row![
                    text(format!("Save #{i}"), text_size,),
                    button!("Load" => {
                        load_state = Some(i);
                    })
                    .padding_horizontal(20.0),
                    button!("Delete" => {
                            delete_state = Some(i);
                    })
                    .padding_horizontal(20.0),
                ]
                .padding_vertical(10.0)
                .boxed()
            })
            .collect();
        if let Some(i) = delete_state {
            self.saved_states.remove(i);
        } else if let Some(i) = load_state {
            self.load_state(i);
        }

        let tas_ui = stack![
            text(
                if self.paused {
                    "Paused".to_string()
                } else if let Some(replay) = &self.replay {
                    format!("Replay frame {}", replay.frame)
                } else {
                    "Recording".to_string()
                },
                text_size
            )
            .align(vec2(1.0, 0.9)),
            slider("Time scale", 0.0..=10.0, &mut self.time_scale, text_size).align(vec2(0.5, 1.0)),
            column![
                text(self.save_file.clone(), text_size),
                row![
                    button!("Save run" => {
                        if let Err(err) = self.save_run(&self.save_file) {
                            log::error!("Failed to save run: {err}");
                        }
                    }),
                    button!("Start replay" => {
                        if let Err(err) = self.load_run(&self.save_file.clone()) {
                            log::error!("Failed to load run: {err}");
                        }
                    }),
                ]
            ]
            .align(vec2(0.0, 0.0)),
            column({
                saved_states.push(
                    button!("Save state" => {
                        self.save_state();
                    })
                    .boxed(),
                );
                saved_states
            })
            .align(vec2(1.0, 0.0))
            .padding_bottom(200.0)
        ]
        .uniform_padding(30.0);

        Box::new(stack(vec![self.game.ui(cx), Box::new(tas_ui)]))
    }
}
