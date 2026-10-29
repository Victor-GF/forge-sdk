//! Log simples em stderr. Formatação fica com quem chama (`&format!(...)`).

pub fn log_info(msg: &str) {
    eprintln!("[info] {msg}");
}

pub fn log_aviso(msg: &str) {
    eprintln!("[aviso] {msg}");
}

pub fn log_erro(msg: &str) {
    eprintln!("[erro] {msg}");
}
