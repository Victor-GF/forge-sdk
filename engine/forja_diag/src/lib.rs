//! Diagnóstico: log com breadcrumbs e crash report. Não aloca.
//!
//! - `log`: linhas em stderr + um ring buffer fixo (breadcrumbs) que vai no crash report.
//! - `crash`: relatório em `<dir>/crash_AAAA-MM-DD_HHMMSS.txt` para panic, exceções do SO
//!   (access violation, stack overflow...) e erros fatais da engine (OOM de arena, alocação
//!   com o alocador global travado). Montado num buffer estático reservado de antemão, para
//!   funcionar mesmo sem memória livre.
//!
//! Exceção documentada ao "sem estado global": log e crash são do processo inteiro.
//! O estado fica em statics com atômicos ou spin locks, nunca alocado.

pub mod crash;
pub mod log;
mod spin_lock;
