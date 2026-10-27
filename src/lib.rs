//! forja — a data-oriented game engine core.
//!
//! The pieces, from the bottom up:
//!
//! - [`jobs`]: fixed worker pool. Scoped jobs that can borrow from the stack, `par_for`/`par_zip`
//!   for data-parallel loops over SoA slices, and background jobs used to pipeline frames.
//! - [`world`]: entities stored in tables (one table per component set, one `Vec<T>` per component),
//!   resources, and command buffers for structural changes.
//! - [`schedule`]: systems are plain functions that declare what they read and write. A schedule
//!   turns those declarations into a dependency graph and runs independent systems in parallel.
//! - [`frame`]: the frame model. Fixed-rate simulation ticks, an extract sync point, and rendering
//!   pipelined one frame behind the simulation through ownership hand-off (no shared mutable state).
//! - [`simd`]: 8-lane `F32x8`/`I32x8` used by the hot loops (AVX2+FMA, or a portable fallback).
//!   On x86-64 this repo targets x86-64-v3; [`cpu`] checks the machine at startup.
//! - [`physics`], [`render`]: engine features written with the same tools a game uses.

pub mod alloc;
pub mod arena;
pub mod cpu;
pub mod frame;
pub mod image;
pub mod jobs;
pub mod math;
pub mod physics;
pub mod platform;
pub mod profile;
pub mod render;
pub mod scene;
pub mod schedule;
pub mod simd;
pub mod telemetry;
pub mod world;

pub mod prelude {
    pub use crate::arena::FrameArena;
    pub use crate::frame::{FrameClock, FrameInfo, FramePipeline, Renderer};
    pub use crate::jobs::{JobHandle, JobSystem, Jobs};
    pub use crate::math::{Mat4, Quat, Vec3, Vec4, vec3};
    pub use crate::physics::{Body, BodyId, PhysicsWorld, Rigid, RigidBody, RigidId, StaticBox};
    pub use crate::render::{Mesh, MeshId, SoftwareRenderer};
    pub use crate::scene::{Camera, Renderable, Sun, Transform};
    pub use crate::schedule::{Schedule, SystemCtx, system};
    pub use crate::world::{Commands, Entity, World};
}
