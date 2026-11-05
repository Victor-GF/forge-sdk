//! Input: teclado, mouse e gamepads, como estado por frame.
//!
//! Nao ha fila de eventos: cada frame o jogo recebe um retrato. Para cada
//! botao (tecla, botao do mouse ou do gamepad), o retrato diz se esta
//! pressionado e quantas vezes mudou de estado no frame. Assim um toque mais
//! rapido que um frame nao se perde, e "acabou de apertar" e uma conta, nao
//! um evento.
//!
//! Teclado e mouse vem dos eventos da janela ([`Window::window_input`]);
//! gamepads sao lidos direto do dispositivo ([`Gamepads::gamepads_update`]).
//!
//! Todos os tipos sao `#[repr(C)]` e sem ponteiros: podem ser passados como
//! estao para o codigo do jogo numa biblioteca dinamica (hot reload).
//!
//! [`Window::window_input`]: crate::window::Window::window_input

use core::mem;

/// Um botao num frame.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Button {
    /// Pressionado no fim do frame.
    pub down: bool,
    /// Quantas vezes mudou de estado no frame. Com 2 ou mais, um toque
    /// inteiro (apertar e soltar) coube dentro do frame.
    pub transitions: u8,
}

impl Button {
    /// Foi apertado pelo menos uma vez neste frame.
    #[inline]
    pub fn pressed(self) -> bool {
        self.transitions >= 2 || (self.transitions == 1 && self.down)
    }

    /// Foi solto pelo menos uma vez neste frame.
    #[inline]
    pub fn released(self) -> bool {
        self.transitions >= 2 || (self.transitions == 1 && !self.down)
    }

    /// Registra o estado novo; repetir o mesmo estado nao conta transicao.
    #[inline]
    pub fn set(&mut self, down: bool) {
        if self.down != down {
            self.down = down;
            self.transitions = self.transitions.saturating_add(1);
        }
    }

    /// Comeca um frame novo: mantem `down`, zera `transitions`.
    #[inline]
    pub fn begin_frame(&mut self) {
        self.transitions = 0;
    }
}

// ---- Teclado ----------------------------------------------------------------

/// Tecla pela posicao fisica, com o nome que ela tem no teclado US. `Key::W`
/// e a tecla acima de `S` em qualquer layout (no AZERTY, e a que mostra `Z`):
/// e o que controles de jogo querem. Texto digitado e outra coisa, ainda nao
/// coberta.
#[rustfmt::skip]
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    A, B, C, D, E, F, G, H, I, J, K, L, M,
    N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
    Digit0, Digit1, Digit2, Digit3, Digit4,
    Digit5, Digit6, Digit7, Digit8, Digit9,
    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
    Escape, Enter, Tab, Backspace, Space,
    Minus, Equals, Grave,
    Up, Down, Left, Right,
    Insert, Delete, Home, End, PageUp, PageDown,
    LeftShift, RightShift, LeftControl, RightControl, LeftAlt, RightAlt,
}

pub const KEY_COUNT: usize = Key::RightAlt as usize + 1;

impl Key {
    /// A tecla de indice `index` em [`Keyboard::keys`].
    #[inline]
    pub fn from_index(index: usize) -> Option<Key> {
        if index < KEY_COUNT {
            // `repr(u8)` sem valores explicitos: as variantes sao 0..KEY_COUNT.
            Some(unsafe { mem::transmute::<u8, Key>(index as u8) })
        } else {
            None
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Keyboard {
    /// Indexado por `Key as usize`.
    pub keys: [Button; KEY_COUNT],
}

impl Keyboard {
    #[inline]
    pub fn key(&self, key: Key) -> Button {
        self.keys[key as usize]
    }
}

impl Default for Keyboard {
    fn default() -> Keyboard {
        Keyboard { keys: [Button::default(); KEY_COUNT] }
    }
}

// ---- Mouse ------------------------------------------------------------------

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    /// Botoes laterais: voltar e avancar.
    X1,
    X2,
}

pub const MOUSE_BUTTON_COUNT: usize = MouseButton::X2 as usize + 1;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Mouse {
    /// Posicao do cursor na area de desenho, em pixels, com y para baixo.
    /// Pode sair da janela enquanto um botao estiver pressionado.
    pub x: i32,
    pub y: i32,
    /// Movimento do dispositivo no frame, nos eixos de `x`/`y`, cru: sem
    /// aceleracao do SO e sem parar na borda da tela. E o que uma camera em
    /// primeira pessoa quer. A unidade e a do sensor, nao pixel.
    pub delta_x: i32,
    pub delta_y: i32,
    /// Giro da roda no frame, em cliques; positivo e para longe do usuario.
    pub wheel: f32,
    /// Indexado por `MouseButton as usize`.
    pub buttons: [Button; MOUSE_BUTTON_COUNT],
}

impl Mouse {
    #[inline]
    pub fn button(&self, button: MouseButton) -> Button {
        self.buttons[button as usize]
    }
}

// ---- Gamepad ----------------------------------------------------------------

/// Botoes pela posicao, para valer em qualquer controle. Os quatro da face
/// sao `South` (Xbox A, PlayStation X), `East` (B, O), `West` (X, quadrado) e
/// `North` (Y, triangulo).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GamepadButton {
    South,
    East,
    West,
    North,
    LeftShoulder,
    RightShoulder,
    /// Clique do analogico.
    LeftStick,
    RightStick,
    /// Xbox: View; PlayStation: Share/Create.
    Back,
    /// Xbox: Menu; PlayStation: Options.
    Start,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
}

pub const GAMEPAD_BUTTON_COUNT: usize = GamepadButton::DpadRight as usize + 1;

/// Quantos gamepads sao lidos.
pub const GAMEPAD_COUNT: usize = 4;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Gamepad {
    /// Sem controle neste slot, o resto fica zerado.
    pub connected: bool,
    /// Indexado por `GamepadButton as usize`.
    pub buttons: [Button; GAMEPAD_BUTTON_COUNT],
    /// Analogicos de -1 a 1, com y para cima, ja sem a zona morta (o
    /// tremor de um analogico solto vira 0).
    pub left_x: f32,
    pub left_y: f32,
    pub right_x: f32,
    pub right_y: f32,
    /// Gatilhos de 0 a 1, ja sem a zona morta.
    pub left_trigger: f32,
    pub right_trigger: f32,
}

impl Gamepad {
    #[inline]
    pub fn button(&self, button: GamepadButton) -> Button {
        self.buttons[button as usize]
    }
}

/// Todo o input de um frame, junto: o que o jogo recebe.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Input {
    pub keyboard: Keyboard,
    pub mouse: Mouse,
    pub gamepads: [Gamepad; GAMEPAD_COUNT],
}

pub trait Gamepads {
    /// Le todos os gamepads. `pads` precisa ser o mesmo array a cada frame:
    /// as transicoes saem da comparacao com o frame anterior. Um slot vazio e
    /// consultado de novo so de tempos em tempos, porque perguntar por
    /// controle desconectado e caro em algumas plataformas.
    fn gamepads_update(pads: &mut [Gamepad; GAMEPAD_COUNT]);
}
