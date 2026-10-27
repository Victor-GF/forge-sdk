//! Common components and resources shared by gameplay, physics and rendering.

use crate::{
    math::{Quat, Vec3, vec3},
    render::MeshId,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub pos: Vec3,
    pub rot: Quat,
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Transform {
        Transform { pos: Vec3::ZERO, rot: Quat::IDENTITY, scale: Vec3::ONE }
    }
}

impl Transform {
    pub fn from_pos(pos: Vec3) -> Transform {
        Transform { pos, ..Transform::default() }
    }
    pub fn with_scale(mut self, scale: Vec3) -> Transform {
        self.scale = scale;
        self
    }
    pub fn with_rot(mut self, rot: Quat) -> Transform {
        self.rot = rot;
        self
    }
}

/// Something the renderer draws. `color` is linear albedo.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Renderable {
    pub mesh: MeshId,
    pub color: Vec3,
}

/// Resource: the view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub eye: Vec3,
    pub target: Vec3,
    pub fov_y: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for Camera {
    fn default() -> Camera {
        Camera { eye: vec3(0.0, 5.0, 15.0), target: Vec3::ZERO, fov_y: 1.0, near: 0.1, far: 200.0 }
    }
}

/// Resource: directional light with a shadow map covering a sphere around `shadow_center`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sun {
    /// Direction the light travels (from the sun towards the ground).
    pub dir: Vec3,
    pub color: Vec3,
    pub ambient: f32,
    pub shadow_center: Vec3,
    pub shadow_radius: f32,
}

impl Default for Sun {
    fn default() -> Sun {
        Sun {
            dir: vec3(-0.4, -1.0, -0.3).normalize(),
            color: vec3(1.0, 0.95, 0.85),
            ambient: 0.35,
            shadow_center: Vec3::ZERO,
            shadow_radius: 20.0,
        }
    }
}
