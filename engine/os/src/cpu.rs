//! CPU: topologia da maquina, para dimensionar trabalho com dados em vez de
//! chute.
//!
//! Para que serve cada numero:
//! - `physical_cores` x `logical_processors`: com SMT (Hyper-Threading), duas
//!   threads logicas dividem um nucleo, suas unidades de execucao e seus L1 e
//!   L2. Trabalho pesado de calculo rende bem menos que o dobro nelas: um
//!   pool so de calculo pode querer um worker por nucleo fisico.
//! - `performance_cores`: em CPUs hibridas (Intel P/E), os nucleos de
//!   eficiencia sao mais lentos. Sem hibrido, e igual a `physical_cores`.
//! - Caches: blocos de trabalho que cabem no L2 de um nucleo nao disputam
//!   memoria com os vizinhos. `l3_count` maior que 1 (Ryzen com mais de um
//!   CCD) quer dizer que nem todos os nucleos veem o mesmo L3.
//! - `cache_line`: confere `forge_core::cache::CACHE_LINE` na maquina real.

/// Topologia da CPU. Tamanhos em bytes; zero quando o SO nao informou.
#[derive(Clone, Copy, Debug, Default)]
pub struct CpuTopology {
    pub physical_cores: u32,
    pub logical_processors: u32,
    /// Nucleos da classe mais rapida. Igual a `physical_cores` fora de CPUs
    /// hibridas.
    pub performance_cores: u32,
    /// Tamanho da linha de cache do L1 de dados.
    pub cache_line: u32,
    /// L1 de dados de um nucleo.
    pub l1_data: u32,
    /// Um L2 (por nucleo ou por grupo de nucleos, conforme a CPU).
    pub l2: u32,
    /// Um L3, e quantos L3 separados existem.
    pub l3: u32,
    pub l3_count: u32,
}

pub trait Cpu {
    /// Consulta a topologia. Custa uma chamada ao SO e um buffer temporario:
    /// para a inicializacao, nao para todo frame. `None` se falhar.
    fn cpu_topology() -> Option<CpuTopology>;
}
