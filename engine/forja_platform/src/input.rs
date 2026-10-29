//! Estado de teclado e mouse. Funções puras: a plataforma só traduz mensagens do SO
//! para `input_*_evento`.
//!
//! Teclas usam os códigos virtuais do Windows. Letras e dígitos são o próprio ASCII
//! maiúsculo: `b'W'`, `b'0'`. As demais têm constante `TECLA_*`.

pub const TECLAS_MAX: usize = 256;
pub const MOUSE_BOTOES_MAX: usize = 3;

pub const MOUSE_ESQUERDO: u8 = 0;
pub const MOUSE_DIREITO: u8 = 1;
pub const MOUSE_MEIO: u8 = 2;

pub const TECLA_BACKSPACE: u8 = 0x08;
pub const TECLA_TAB: u8 = 0x09;
pub const TECLA_ENTER: u8 = 0x0D;
pub const TECLA_SHIFT: u8 = 0x10;
pub const TECLA_CTRL: u8 = 0x11;
pub const TECLA_ALT: u8 = 0x12;
pub const TECLA_ESC: u8 = 0x1B;
pub const TECLA_ESPACO: u8 = 0x20;
pub const TECLA_ESQUERDA: u8 = 0x25;
pub const TECLA_CIMA: u8 = 0x26;
pub const TECLA_DIREITA: u8 = 0x27;
pub const TECLA_BAIXO: u8 = 0x28;
pub const TECLA_F1: u8 = 0x70;
pub const TECLA_F2: u8 = 0x71;
pub const TECLA_F3: u8 = 0x72;
pub const TECLA_F4: u8 = 0x73;
pub const TECLA_F5: u8 = 0x74;
pub const TECLA_F6: u8 = 0x75;
pub const TECLA_F7: u8 = 0x76;
pub const TECLA_F8: u8 = 0x77;
pub const TECLA_F9: u8 = 0x78;
pub const TECLA_F10: u8 = 0x79;
pub const TECLA_F11: u8 = 0x7A;
pub const TECLA_F12: u8 = 0x7B;

/// `pressionou`/`soltou` valem só no frame em que aconteceram; `baixo` é o estado atual.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Input {
    pub tecla_baixo: [bool; TECLAS_MAX],
    pub tecla_pressionou: [bool; TECLAS_MAX],
    pub tecla_soltou: [bool; TECLAS_MAX],
    pub mouse_baixo: [bool; MOUSE_BOTOES_MAX],
    pub mouse_pressionou: [bool; MOUSE_BOTOES_MAX],
    pub mouse_soltou: [bool; MOUSE_BOTOES_MAX],
    /// Posição em pixels, relativa ao canto superior esquerdo da área cliente.
    pub mouse_x: i32,
    pub mouse_y: i32,
    /// Movimento acumulado no frame.
    pub mouse_dx: i32,
    pub mouse_dy: i32,
    /// Roda acumulada no frame (120 = um "clique" para frente).
    pub roda: i32,
}

pub const INPUT_VAZIO: Input = Input {
    tecla_baixo: [false; TECLAS_MAX],
    tecla_pressionou: [false; TECLAS_MAX],
    tecla_soltou: [false; TECLAS_MAX],
    mouse_baixo: [false; MOUSE_BOTOES_MAX],
    mouse_pressionou: [false; MOUSE_BOTOES_MAX],
    mouse_soltou: [false; MOUSE_BOTOES_MAX],
    mouse_x: 0,
    mouse_y: 0,
    mouse_dx: 0,
    mouse_dy: 0,
    roda: 0,
};

// ---------------------------------------------------------------- consultas

pub fn input_tecla_baixo(input: &Input, tecla: u8) -> bool {
    input.tecla_baixo.get(usize::from(tecla)).copied().unwrap_or(false)
}

pub fn input_tecla_pressionou(input: &Input, tecla: u8) -> bool {
    input.tecla_pressionou.get(usize::from(tecla)).copied().unwrap_or(false)
}

pub fn input_tecla_soltou(input: &Input, tecla: u8) -> bool {
    input.tecla_soltou.get(usize::from(tecla)).copied().unwrap_or(false)
}

pub fn input_mouse_baixo(input: &Input, botao: u8) -> bool {
    input.mouse_baixo.get(usize::from(botao)).copied().unwrap_or(false)
}

pub fn input_mouse_pressionou(input: &Input, botao: u8) -> bool {
    input.mouse_pressionou.get(usize::from(botao)).copied().unwrap_or(false)
}

pub fn input_mouse_soltou(input: &Input, botao: u8) -> bool {
    input.mouse_soltou.get(usize::from(botao)).copied().unwrap_or(false)
}

// ---------------------------------------------------------------- eventos

/// Limpa o que só vale por um frame. Chamado pela plataforma antes de ler mensagens.
pub fn input_frame_inicio(input: &mut Input) {
    input.tecla_pressionou = [false; TECLAS_MAX];
    input.tecla_soltou = [false; TECLAS_MAX];
    input.mouse_pressionou = [false; MOUSE_BOTOES_MAX];
    input.mouse_soltou = [false; MOUSE_BOTOES_MAX];
    input.mouse_dx = 0;
    input.mouse_dy = 0;
    input.roda = 0;
}

/// Atualiza o estado do botão `i` e marca a transição. Índice fora do array é ignorado.
fn botao_evento(baixo: &mut [bool], pressionou: &mut [bool], soltou: &mut [bool], i: usize, agora: bool) {
    let Some(estado) = baixo.get_mut(i) else {
        return;
    };
    let estava = *estado;
    *estado = agora;
    if let Some(v) = pressionou.get_mut(i) {
        *v |= agora && !estava;
    }
    if let Some(v) = soltou.get_mut(i) {
        *v |= !agora && estava;
    }
}

/// Repetição automática do SO (tecla já baixa) não gera novo `pressionou`.
pub fn input_tecla_evento(input: &mut Input, tecla: u8, baixo: bool) {
    botao_evento(
        &mut input.tecla_baixo,
        &mut input.tecla_pressionou,
        &mut input.tecla_soltou,
        usize::from(tecla),
        baixo,
    );
}

pub fn input_mouse_evento(input: &mut Input, botao: u8, baixo: bool) {
    botao_evento(
        &mut input.mouse_baixo,
        &mut input.mouse_pressionou,
        &mut input.mouse_soltou,
        usize::from(botao),
        baixo,
    );
}

pub fn input_mouse_mover(input: &mut Input, x: i32, y: i32) {
    input.mouse_dx = input.mouse_dx.saturating_add(x.saturating_sub(input.mouse_x));
    input.mouse_dy = input.mouse_dy.saturating_add(y.saturating_sub(input.mouse_y));
    input.mouse_x = x;
    input.mouse_y = y;
}

pub fn input_roda_evento(input: &mut Input, delta: i32) {
    input.roda = input.roda.saturating_add(delta);
}

/// Solta tudo que está baixo (gerando `soltou`). Usado quando a janela perde o foco,
/// para nenhuma tecla ficar "presa".
pub fn input_soltar_tudo(input: &mut Input) {
    for (baixo, soltou) in input.tecla_baixo.iter_mut().zip(input.tecla_soltou.iter_mut()) {
        if *baixo {
            *baixo = false;
            *soltou = true;
        }
    }
    for (baixo, soltou) in input.mouse_baixo.iter_mut().zip(input.mouse_soltou.iter_mut()) {
        if *baixo {
            *baixo = false;
            *soltou = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressionou_so_no_primeiro_frame() {
        let mut input = INPUT_VAZIO;
        input_frame_inicio(&mut input);
        input_tecla_evento(&mut input, b'W', true);
        assert!(input_tecla_pressionou(&input, b'W'));
        assert!(input_tecla_baixo(&input, b'W'));

        // próximo frame: repetição do SO não conta como novo toque
        input_frame_inicio(&mut input);
        input_tecla_evento(&mut input, b'W', true);
        assert!(!input_tecla_pressionou(&input, b'W'));
        assert!(input_tecla_baixo(&input, b'W'));

        input_frame_inicio(&mut input);
        input_tecla_evento(&mut input, b'W', false);
        assert!(input_tecla_soltou(&input, b'W'));
        assert!(!input_tecla_baixo(&input, b'W'));
    }

    #[test]
    fn perder_foco_solta_teclas() {
        let mut input = INPUT_VAZIO;
        input_tecla_evento(&mut input, TECLA_ESPACO, true);
        input_mouse_evento(&mut input, MOUSE_ESQUERDO, true);
        input_frame_inicio(&mut input);
        input_soltar_tudo(&mut input);
        assert!(!input_tecla_baixo(&input, TECLA_ESPACO));
        assert!(input_tecla_soltou(&input, TECLA_ESPACO));
        assert!(input_mouse_soltou(&input, MOUSE_ESQUERDO));
    }

    #[test]
    fn mouse_acumula_movimento_no_frame() {
        let mut input = INPUT_VAZIO;
        input_mouse_mover(&mut input, 10, 10);
        input_frame_inicio(&mut input);
        input_mouse_mover(&mut input, 12, 7);
        input_mouse_mover(&mut input, 15, 9);
        assert_eq!((input.mouse_dx, input.mouse_dy), (5, -1));
        assert_eq!((input.mouse_x, input.mouse_y), (15, 9));
    }

    #[test]
    fn botao_invalido_e_ignorado() {
        let mut input = INPUT_VAZIO;
        input_mouse_evento(&mut input, 200, true);
        assert!(!input_mouse_baixo(&input, 200));
    }
}
