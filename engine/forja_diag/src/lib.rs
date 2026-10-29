//! Diagnóstico: log com rastro e crash report. Não aloca.
//!
//! - `log`: linhas em stderr + um ring buffer fixo (o "rastro") que vai no crash report.
//! - `crash`: relatório em `<pasta>/crash_AAAA-MM-DD_HHMMSS.txt` para panic, exceções do SO
//!   (access violation, stack overflow...) e erros fatais da engine (OOM de arena, alocação
//!   com o alocador global travado). Montado num buffer estático reservado de antemão, para
//!   funcionar mesmo sem memória livre.
//!
//! Exceção documentada ao "sem estado global": log e crash são do processo inteiro.
//! O estado fica em statics com atômicos ou travas de giro (spin), nunca alocado.

pub mod crash;
pub mod log;
mod trava;
