//! Estado de teclado e mouse. A plataforma só traduz mensagens do SO para os métodos
//! `*_event`; o jogo consulta com `key_down`, `key_pressed` etc.
//!
//! `Key` usa os códigos virtuais do Windows. Letras e dígitos são o próprio ASCII
//! maiúsculo: `Key(b'W')`, `Key(b'0')`. As demais têm constante (`Key::ESCAPE`...).

pub const KEY_COUNT: usize = 256;
pub const MOUSE_BUTTON_COUNT: usize = 3;

/// Tecla (código virtual do Windows). Todo `u8` é um índice válido da tabela.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key(pub u8);

impl Key {
    pub const BACKSPACE: Key = Key(0x08);
    pub const TAB: Key = Key(0x09);
    pub const ENTER: Key = Key(0x0D);
    pub const SHIFT: Key = Key(0x10);
    pub const CONTROL: Key = Key(0x11);
    pub const ALT: Key = Key(0x12);
    pub const ESCAPE: Key = Key(0x1B);
    pub const SPACE: Key = Key(0x20);
    pub const LEFT: Key = Key(0x25);
    pub const UP: Key = Key(0x26);
    pub const RIGHT: Key = Key(0x27);
    pub const DOWN: Key = Key(0x28);
    pub const F1: Key = Key(0x70);
    pub const F2: Key = Key(0x71);
    pub const F3: Key = Key(0x72);
    pub const F4: Key = Key(0x73);
    pub const F5: Key = Key(0x74);
    pub const F6: Key = Key(0x75);
    pub const F7: Key = Key(0x76);
    pub const F8: Key = Key(0x77);
    pub const F9: Key = Key(0x78);
    pub const F10: Key = Key(0x79);
    pub const F11: Key = Key(0x7A);
    pub const F12: Key = Key(0x7B);
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left = 0,
    Right = 1,
    Middle = 2,
}

/// `pressed`/`released` valem só no frame em que aconteceram; `down` é o estado atual.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Input {
    keys_down: [bool; KEY_COUNT],
    keys_pressed: [bool; KEY_COUNT],
    keys_released: [bool; KEY_COUNT],
    mouse_down: [bool; MOUSE_BUTTON_COUNT],
    mouse_pressed: [bool; MOUSE_BUTTON_COUNT],
    mouse_released: [bool; MOUSE_BUTTON_COUNT],
    /// Posição em pixels, relativa ao canto superior esquerdo da área cliente.
    pub mouse_x: i32,
    pub mouse_y: i32,
    /// Movimento acumulado no frame.
    pub mouse_dx: i32,
    pub mouse_dy: i32,
    /// Roda acumulada no frame (120 = um "clique" para frente).
    pub wheel: i32,
}

/// Atualiza o estado do botão `i` e marca a transição. Índice fora do array é ignorado.
fn button_event(down: &mut [bool], pressed: &mut [bool], released: &mut [bool], i: usize, now: bool) {
    let Some(state) = down.get_mut(i) else {
        return;
    };
    let was = *state;
    *state = now;
    if let Some(v) = pressed.get_mut(i) {
        *v |= now && !was;
    }
    if let Some(v) = released.get_mut(i) {
        *v |= !now && was;
    }
}

impl Input {
    pub const EMPTY: Input = Input {
        keys_down: [false; KEY_COUNT],
        keys_pressed: [false; KEY_COUNT],
        keys_released: [false; KEY_COUNT],
        mouse_down: [false; MOUSE_BUTTON_COUNT],
        mouse_pressed: [false; MOUSE_BUTTON_COUNT],
        mouse_released: [false; MOUSE_BUTTON_COUNT],
        mouse_x: 0,
        mouse_y: 0,
        mouse_dx: 0,
        mouse_dy: 0,
        wheel: 0,
    };

    // ------------------------------------------------------------ consultas

    pub fn key_down(&self, k: Key) -> bool {
        self.keys_down.get(usize::from(k.0)).copied().unwrap_or(false)
    }

    pub fn key_pressed(&self, k: Key) -> bool {
        self.keys_pressed.get(usize::from(k.0)).copied().unwrap_or(false)
    }

    pub fn key_released(&self, k: Key) -> bool {
        self.keys_released.get(usize::from(k.0)).copied().unwrap_or(false)
    }

    pub fn mouse_down(&self, b: MouseButton) -> bool {
        self.mouse_down.get(b as usize).copied().unwrap_or(false)
    }

    pub fn mouse_pressed(&self, b: MouseButton) -> bool {
        self.mouse_pressed.get(b as usize).copied().unwrap_or(false)
    }

    pub fn mouse_released(&self, b: MouseButton) -> bool {
        self.mouse_released.get(b as usize).copied().unwrap_or(false)
    }

    // ------------------------------------------------------------ eventos

    /// Limpa o que só vale por um frame. Chamado pela plataforma antes de ler mensagens.
    pub fn begin_frame(&mut self) {
        self.keys_pressed = [false; KEY_COUNT];
        self.keys_released = [false; KEY_COUNT];
        self.mouse_pressed = [false; MOUSE_BUTTON_COUNT];
        self.mouse_released = [false; MOUSE_BUTTON_COUNT];
        self.mouse_dx = 0;
        self.mouse_dy = 0;
        self.wheel = 0;
    }

    /// Repetição automática do SO (tecla já baixa) não gera novo `pressed`.
    pub fn key_event(&mut self, k: Key, down: bool) {
        button_event(
            &mut self.keys_down,
            &mut self.keys_pressed,
            &mut self.keys_released,
            usize::from(k.0),
            down,
        );
    }

    pub fn mouse_button_event(&mut self, b: MouseButton, down: bool) {
        button_event(
            &mut self.mouse_down,
            &mut self.mouse_pressed,
            &mut self.mouse_released,
            b as usize,
            down,
        );
    }

    pub fn mouse_move(&mut self, x: i32, y: i32) {
        self.mouse_dx = self.mouse_dx.saturating_add(x.saturating_sub(self.mouse_x));
        self.mouse_dy = self.mouse_dy.saturating_add(y.saturating_sub(self.mouse_y));
        self.mouse_x = x;
        self.mouse_y = y;
    }

    pub fn wheel_event(&mut self, delta: i32) {
        self.wheel = self.wheel.saturating_add(delta);
    }

    /// Solta tudo que está baixo (gerando `released`). Usado quando a janela perde o foco,
    /// para nenhuma tecla ficar "presa".
    pub fn release_all(&mut self) {
        for (down, released) in self.keys_down.iter_mut().zip(self.keys_released.iter_mut()) {
            if *down {
                *down = false;
                *released = true;
            }
        }
        for (down, released) in self.mouse_down.iter_mut().zip(self.mouse_released.iter_mut()) {
            if *down {
                *down = false;
                *released = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: Key = Key(b'W');

    #[test]
    fn pressed_only_on_first_frame() {
        let mut input = Input::EMPTY;
        input.begin_frame();
        input.key_event(W, true);
        assert!(input.key_pressed(W));
        assert!(input.key_down(W));

        // próximo frame: repetição do SO não conta como novo toque
        input.begin_frame();
        input.key_event(W, true);
        assert!(!input.key_pressed(W));
        assert!(input.key_down(W));

        input.begin_frame();
        input.key_event(W, false);
        assert!(input.key_released(W));
        assert!(!input.key_down(W));
    }

    #[test]
    fn losing_focus_releases_keys() {
        let mut input = Input::EMPTY;
        input.key_event(Key::SPACE, true);
        input.mouse_button_event(MouseButton::Left, true);
        input.begin_frame();
        input.release_all();
        assert!(!input.key_down(Key::SPACE));
        assert!(input.key_released(Key::SPACE));
        assert!(input.mouse_released(MouseButton::Left));
    }

    #[test]
    fn mouse_motion_accumulates_per_frame() {
        let mut input = Input::EMPTY;
        input.mouse_move(10, 10);
        input.begin_frame();
        input.mouse_move(12, 7);
        input.mouse_move(15, 9);
        assert_eq!((input.mouse_dx, input.mouse_dy), (5, -1));
        assert_eq!((input.mouse_x, input.mouse_y), (15, 9));
    }

    #[test]
    fn every_mouse_button_has_a_slot() {
        let mut input = Input::EMPTY;
        for b in [MouseButton::Left, MouseButton::Right, MouseButton::Middle] {
            input.mouse_button_event(b, true);
            assert!(input.mouse_down(b));
        }
    }
}
