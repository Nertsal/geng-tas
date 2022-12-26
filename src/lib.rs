use geng::prelude::{ugli, Geng};

/// A wrapper for a game that implements TAS functionality:
/// save states, slow motion, input replay.
pub struct Tas<T: Clone> {
    geng: Geng,
    /// The game state that is manipulated.
    game: T,
    /// Multiplier for `delta_time`, used for slow-motion.
    time_scale: f64,
    /// The expected time between fixed updates.
    fixed_delta_time: f64,
    /// The time until next the fixed update (if queued).
    next_fixed_update: Option<f64>,
    /// All saved states.
    saved_states: Vec<T>,
    /// A temporary storage for states. Primarily used right after a state is loaded
    /// to give an opportunity to revert it.
    temp_state: Option<T>,
}

impl<T: Clone> Tas<T> {
    pub fn new(game: T, geng: &Geng) -> Self {
        Self {
            geng: geng.clone(),
            game,
            time_scale: 1.0,
            fixed_delta_time: 1.0,
            next_fixed_update: None,
            saved_states: Vec::new(),
            temp_state: None,
        }
    }

    /// Saves the current game state.
    fn save_state(&mut self) {
        self.saved_states.push(self.game.clone());
    }

    /// Attempts to load the saved state by index.
    /// If such a state is not found, nothing happens.
    fn load_state(&mut self, index: usize) {
        // Get the state by index
        if let Some(state) = self.saved_states.get(index) {
            self.load(state.clone());
        }
    }

    /// Loads the given state. Puts the old one in the temporary slot.
    fn load(&mut self, mut state: T) {
        // Swap with the current state
        std::mem::swap(&mut state, &mut self.game);
        // Save the old state to `temp_state`
        self.temp_state = Some(state);
    }

    /// Restores the temporarily saved state if it exists.
    fn restore_temporary(&mut self) {
        if let Some(state) = self.temp_state.take() {
            self.load(state);
        }
    }

    /// Changes the time scale by the given `delta`.
    /// The final time scale is clamped between 0 and 2.
    fn change_time_scale(&mut self, delta: f64) {
        self.set_time_scale(self.time_scale + delta);
    }

    /// Set the time scale to the given `value` clamped between 0 and 2.
    fn set_time_scale(&mut self, value: f64) {
        self.time_scale = value.clamp(0.0, 2.0);
    }

    /// Handle the `geng::KeyDown { key: geng::Key::Num<num> }` event.
    fn num_down(&mut self, mut num: usize) {
        if num == 0 {
            num = 10;
        }
        if self.geng.window().is_key_pressed(geng::Key::L) {
            // Load state
            num -= 1;
            self.load_state(num);
        } else {
            // Set time scale
            self.set_time_scale(num as f64 * 0.1);
        }
    }
}

impl<T: geng::State + Clone> geng::State for Tas<T> {
    fn draw(&mut self, framebuffer: &mut ugli::Framebuffer) {
        self.game.draw(framebuffer);
    }

    fn update(&mut self, delta_time: f64) {
        let delta_time = delta_time * self.time_scale;
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
    }

    fn fixed_update(&mut self, delta_time: f64) {
        self.fixed_delta_time = delta_time;
    }

    fn handle_event(&mut self, event: geng::Event) {
        let window = self.geng.window();
        if window.is_key_pressed(geng::Key::LAlt) {
            // Capture the event
            match event {
                geng::Event::Wheel { delta } => {
                    self.change_time_scale(delta * 0.002);
                }
                geng::Event::KeyDown { key } => match key {
                    // Handle numbers
                    geng::Key::Num0 => self.num_down(0),
                    geng::Key::Num1 => self.num_down(1),
                    geng::Key::Num2 => self.num_down(2),
                    geng::Key::Num3 => self.num_down(3),
                    geng::Key::Num4 => self.num_down(4),
                    geng::Key::Num5 => self.num_down(5),
                    geng::Key::Num6 => self.num_down(6),
                    geng::Key::Num7 => self.num_down(7),
                    geng::Key::Num8 => self.num_down(8),
                    geng::Key::Num9 => self.num_down(9),
                    // Save state
                    geng::Key::S => self.save_state(),
                    // Load temporary
                    geng::Key::T if window.is_key_pressed(geng::Key::L) => self.restore_temporary(),
                    _ => {}
                },
                _ => {}
            }
            return;
        }

        self.game.handle_event(event);
    }
}
