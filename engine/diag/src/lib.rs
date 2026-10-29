//! Diagnóstico: log com breadcrumbs e crash report. Não aloca e não fala com o SO diretamente
//! (tudo passa pelo `forja_os`).
//!
//! - `log`: linhas em stderr + um ring buffer fixo (breadcrumbs) que vai no crash report.
//! - `crash`: relatório em `<dir>/crash_AAAA-MM-DD_HHMMSS.txt` (UTC) para panic, erros do SO
//!   (access violation, stack overflow...) e erros fatais da engine (OOM de arena, heap
//!   proibido). Montado num buffer estático reservado de antemão, para funcionar mesmo sem
//!   memória livre.
//!
//! Exceção documentada ao "sem estado global": log e crash são do processo inteiro.
//! O estado fica em statics com atômicos ou spin locks, nunca alocado.

pub mod crash;
pub mod log;
mod spin_lock;
