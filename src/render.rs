//! Software shading pipeline: strip-binned rasterizer, visibility buffer, shadow map, Blinn-Phong.
//!
//! Per frame (all inside the render job, all data-parallel on the shared pool):
//!
//! 1. **instances** — per instance: frustum test of its bounding sphere and level-of-detail pick
//!    by projected size in pixels (a 5-pixel ball doesn't need 300 triangles).
//! 2. **geometry** — per instance: transform vertices once, assemble and cull triangles, compute
//!    pixel bounds. Output arrays live in the frame arena; each instance writes its own range.
//! 3. **binning** — triangles are counting-sorted into horizontal strips. Each thread counts a
//!    block of triangles into its own histogram row, a prefix sum turns the rows into private
//!    output ranges, then each thread scatters its block: no shared counters, no contention.
//!    A strip is `strip_height` full rows, so a strip's pixels are one contiguous slice of the
//!    framebuffer and can be handed to a thread with `par_zip`, no locks.
//! 4. **shadow pass** — the same rasterizer from the sun, depth only, back faces only (the classic
//!    trick that keeps lit surfaces from shadowing themselves).
//! 5. **main pass** — per strip: rasterize depth + triangle id (visibility buffer), then shade
//!    every pixel exactly once: perspective-correct interpolation, Lambert + Blinn-Phong,
//!    hemispheric ambient, 3x3 PCF shadows, distance fog. Overdraw costs a depth test, not a shade.
//!
//! Each strip lists its triangles in submission order whatever the thread count, so the image is
//! bit-identical with 1 or 32 threads.
//!
//! SIMD: rasterization tests 8 pixels of a row per step (`F32x8` edge functions, masked depth
//! test), and shading runs 8 pixels at a time in structure-of-arrays form, gathering each lane's
//! triangle and vertex attributes (`Vert`/`Tri`/`Instance` are `repr(C)` words for that).

use std::sync::atomic::{AtomicU32, Ordering};

use crate::{
    arena::FrameArena,
    frame::{FrameInfo, Renderer},
    jobs::Jobs,
    math::{Mat4, Quat, Vec3, Vec4, vec3},
    profile,
    scene::{Camera, Renderable, Sun, Transform},
    simd::{F32x8, I32x8, M32x8},
    world::World,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct MeshId(pub u32);

/// Indexed triangle mesh, counter-clockwise when seen from outside.
#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub indices: Vec<u32>,
}

impl Mesh {
    /// Unit-radius UV sphere.
    pub fn sphere(segments: u32, rings: u32) -> Mesh {
        let mut m = Mesh::default();
        for r in 0..=rings {
            let theta = std::f32::consts::PI * r as f32 / rings as f32;
            for s in 0..=segments {
                let phi = std::f32::consts::TAU * s as f32 / segments as f32;
                let n = vec3(theta.sin() * phi.cos(), theta.cos(), -theta.sin() * phi.sin());
                m.positions.push(n);
                m.normals.push(n);
            }
        }
        let row = segments + 1;
        for r in 0..rings {
            for s in 0..segments {
                let (a, b) = (r * row + s, (r + 1) * row + s);
                if r != 0 {
                    m.indices.extend([a, b, a + 1]);
                }
                if r != rings - 1 {
                    m.indices.extend([a + 1, b, b + 1]);
                }
            }
        }
        m
    }

    /// Cube from -1 to 1 (scale = half extents), flat normals.
    pub fn cube() -> Mesh {
        let mut m = Mesh::default();
        let faces = [(Vec3::X, Vec3::Y), (-Vec3::X, Vec3::Y), (Vec3::Y, Vec3::Z), (-Vec3::Y, Vec3::Z), (Vec3::Z, Vec3::Y), (-Vec3::Z, Vec3::Y)];
        for (n, up) in faces {
            let right = up.cross(n);
            let base = m.positions.len() as u32;
            for (u, v) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                m.positions.push(n + right * u + up * v);
                m.normals.push(n);
            }
            m.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        m
    }

    /// Cylinder along X from -1 to 1 with radius 1 (a wheel: scale = (half width, r, r)).
    pub fn cylinder(segments: u32) -> Mesh {
        let mut m = Mesh::default();
        let ring = |i: u32| {
            let a = std::f32::consts::TAU * i as f32 / segments as f32;
            vec3(0.0, a.cos(), a.sin())
        };
        // Side: two rings with smooth normals.
        for i in 0..=segments {
            let n = ring(i);
            m.positions.extend([n - Vec3::X, n + Vec3::X]);
            m.normals.extend([n, n]);
        }
        for i in 0..segments {
            let (a0, b0, a1, b1) = (2 * i, 2 * i + 1, 2 * i + 2, 2 * i + 3);
            m.indices.extend([a0, a1, b1, a0, b1, b0]);
        }
        // Caps: a fan each, flat normals.
        for side in [1.0f32, -1.0] {
            let center = m.positions.len() as u32;
            m.positions.push(Vec3::X * side);
            m.normals.push(Vec3::X * side);
            for i in 0..segments {
                m.positions.push(ring(i) + Vec3::X * side);
                m.normals.push(Vec3::X * side);
            }
            for i in 0..segments {
                let (p, q) = (center + 1 + i, center + 1 + (i + 1) % segments);
                m.indices.extend(if side > 0.0 { [center, p, q] } else { [center, q, p] });
            }
        }
        m
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Radius of the bounding sphere around the origin.
    pub fn bound_radius(&self) -> f32 {
        self.positions.iter().map(|p| p.length()).fold(0.0, f32::max)
    }
}

struct MeshEntry {
    mesh: Mesh,
    radius: f32,
    /// (minimum projected radius in pixels, mesh), most detailed first. Empty: no LODs.
    lods: Vec<(f32, u32)>,
}

const NO_MESH: u32 = u32::MAX;

/// One drawable in a snapshot: everything the renderer needs, copied out of the world.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct Instance {
    pub mesh: MeshId,
    pub pos: Vec3,
    pub rot: Quat,
    pub scale: Vec3,
    pub color: Vec3,
}

#[derive(Default)]
pub struct Snapshot {
    pub frame: u64,
    pub camera: Camera,
    pub sun: Sun,
    pub instances: Vec<Instance>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RenderStats {
    pub instances: usize,
    pub triangles: usize,
    /// Triangle-strip pairs after culling (a triangle spanning 3 strips counts 3 times).
    pub binned: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct RenderSettings {
    pub strip_height: usize,
    pub shadow_size: usize,
    pub sky_top: Vec3,
    pub sky_horizon: Vec3,
    pub ground_bounce: Vec3,
    pub fog_density: f32,
    pub shininess: i32,
    pub specular: f32,
}

impl Default for RenderSettings {
    fn default() -> RenderSettings {
        RenderSettings {
            strip_height: 16,
            shadow_size: 1024,
            sky_top: vec3(0.22, 0.38, 0.65),
            sky_horizon: vec3(0.70, 0.78, 0.86),
            ground_bounce: vec3(0.30, 0.26, 0.22),
            fog_density: 0.012,
            shininess: 48,
            specular: 0.35,
        }
    }
}

/// Screen-space vertex. `wpos`/`n` are only filled for the main pass.
#[derive(Clone, Copy, Default)]
#[repr(C)]
struct Vert {
    x: f32,
    y: f32,
    z: f32,
    inv_w: f32,
    wpos: Vec3,
    n: Vec3,
}

/// Triangle after culling, wound so its signed area is positive. `inst == CULLED` means dropped.
#[derive(Clone, Copy, Default)]
#[repr(C)]
struct Tri {
    v: [u32; 3],
    inst: u32,
    min_x: u16,
    max_x: u16,
    min_y: u16,
    max_y: u16,
}

/// Structs made only of 4-byte fields, read as flat word arrays by SIMD gathers.
///
/// # Safety
/// Implementors are `repr(C)`, exactly `WORDS * 4` bytes, 4-byte aligned, without padding.
unsafe trait Words: Copy {
    const WORDS: usize;
}
unsafe impl Words for Vert {
    const WORDS: usize = 10;
}
unsafe impl Words for Tri {
    const WORDS: usize = 6;
}
unsafe impl Words for Instance {
    const WORDS: usize = 14;
}
const _: () = assert!(size_of::<Vert>() == 40 && size_of::<Tri>() == 24 && size_of::<Instance>() == 56);
const INSTANCE_COLOR: i32 = (std::mem::offset_of!(Instance, color) / 4) as i32;

fn as_f32<T: Words>(s: &[T]) -> &[f32] {
    // SAFETY: see `Words`; every bit pattern is a valid f32.
    unsafe { std::slice::from_raw_parts(s.as_ptr().cast(), s.len() * T::WORDS) }
}

fn as_u32<T: Words>(s: &[T]) -> &[u32] {
    // SAFETY: see `Words`.
    unsafe { std::slice::from_raw_parts(s.as_ptr().cast(), s.len() * T::WORDS) }
}

const CULLED: u32 = u32::MAX;
const NO_TRI: u32 = u32::MAX;

/// Persistent binning arrays, reused every frame.
#[derive(Default)]
struct Bins {
    /// One histogram row per block of triangles; after the prefix sum, each row holds that
    /// block's write cursor per strip.
    counts: Vec<u32>,
    /// Strip s owns `items[start[s]..start[s + 1]]`.
    start: Vec<u32>,
    /// Triangle ids. Atomic only so blocks can write their disjoint slots through a shared
    /// reference; stores are relaxed and uncontended (plain moves on x86/ARM).
    items: Vec<AtomicU32>,
}

const BIN_BLOCKS: usize = 64;

pub struct SoftwareRenderer {
    pub width: usize,
    pub height: usize,
    /// 0x00RRGGBB, row-major, top row first.
    pub color: Vec<u32>,
    pub settings: RenderSettings,
    pub stats: RenderStats,
    depth: Vec<f32>,
    vis: Vec<u32>,
    shadow: Vec<f32>,
    meshes: Vec<MeshEntry>,
    arena: FrameArena,
    bins: Bins,
}

impl SoftwareRenderer {
    pub fn new(width: usize, height: usize) -> SoftwareRenderer {
        let settings = RenderSettings::default();
        SoftwareRenderer {
            width,
            height,
            color: vec![0; width * height],
            depth: vec![1.0; width * height],
            vis: vec![NO_TRI; width * height],
            shadow: vec![1.0; settings.shadow_size * settings.shadow_size],
            settings,
            stats: RenderStats::default(),
            meshes: Vec::new(),
            arena: FrameArena::with_capacity(32 << 20),
            bins: Bins::default(),
        }
    }

    pub fn add_mesh(&mut self, mesh: Mesh) -> MeshId {
        self.meshes.push(MeshEntry { radius: mesh.bound_radius(), mesh, lods: Vec::new() });
        MeshId(self.meshes.len() as u32 - 1)
    }

    /// Adds a mesh with levels of detail: `(mesh, min_radius_px)`, most detailed first. Each
    /// frame and pass, an instance draws the first level whose threshold its projected radius
    /// reaches (the last level is the fallback). Returns the id to put in `Renderable`.
    pub fn add_mesh_lods(&mut self, levels: Vec<(Mesh, f32)>) -> MeshId {
        assert!(!levels.is_empty());
        let mut lods = Vec::new();
        for (mesh, min_px) in levels {
            lods.push((min_px, self.add_mesh(mesh).0));
        }
        let base = lods[0].1;
        self.meshes[base as usize].lods = lods;
        MeshId(base)
    }

    /// Heap bytes reserved per part of the renderer.
    pub fn memory(&self) -> Vec<(&'static str, usize)> {
        let meshes = self.meshes.iter().map(|m| m.mesh.positions.capacity() * 24 + m.mesh.indices.capacity() * 4).sum();
        vec![
            ("render.framebuffers", self.color.capacity() * 4 + self.depth.capacity() * 4 + self.vis.capacity() * 4),
            ("render.shadow_map", self.shadow.capacity() * 4),
            ("render.meshes", meshes),
            ("render.arena", self.arena.capacity()),
            ("render.bins", self.bins.counts.capacity() * 4 + self.bins.start.capacity() * 4 + self.bins.items.capacity() * 4),
        ]
    }

    pub fn resize(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
        self.color = vec![0; width * height];
        self.depth = vec![1.0; width * height];
        self.vis = vec![NO_TRI; width * height];
    }

    fn render_frame(&mut self, snap: &Snapshot, jobs: &Jobs) {
        self.arena.reset();
        if self.shadow.len() != self.settings.shadow_size * self.settings.shadow_size {
            self.shadow = vec![1.0; self.settings.shadow_size * self.settings.shadow_size];
        }
        let SoftwareRenderer { width, height, color, settings, stats, depth, vis, shadow, meshes, arena, bins } = self;
        let (w, h, s) = (*width, *height, settings.shadow_size);

        let sun_dir = snap.sun.dir.normalize();
        let shadow_vp = {
            let (c, r) = (snap.sun.shadow_center, snap.sun.shadow_radius);
            let up = if sun_dir.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
            let view = Mat4::look_at_rh(c - sun_dir * (2.0 * r), c, up);
            Mat4::orthographic_rh(-r, r, -r, r, 0.0, 4.0 * r) * view
        };
        let cam = snap.camera;
        let view_proj = Mat4::perspective_rh(cam.fov_y, w as f32 / h as f32, cam.near, cam.far)
            * Mat4::look_at_rh(cam.eye, cam.target, Vec3::Y);

        // Shadow pass: back faces only, depth only.
        {
            let mut _p = profile::scope("render.shadow");
            let pass = Pass {
                view_proj: shadow_vp,
                w: s,
                h: s,
                px_scale: s as f32 * 0.5 / snap.sun.shadow_radius,
                cull: Cull::Front,
                attributes: false,
            };
            let (verts, tris) = geometry(jobs, arena, meshes, &snap.instances, &pass);
            bin(jobs, bins, tris, s, settings.strip_height);
            _p.set_bytes(s * s * 4 + size_of_val(verts) + size_of_val(tris));
            let (verts, tris, bins) = (&*verts, &*tris, &*bins);
            let strip = settings.strip_height;
            jobs.par_zip(&mut shadow[..], s * strip, |offset, depth| {
                let y0 = offset / s;
                let rows = depth.len() / s;
                depth.fill(1.0);
                let mut target = DepthOnly { depth };
                for t in strip_tris(bins, y0 / strip) {
                    raster(&tris[t as usize], verts, y0, rows, s, &mut target);
                }
            });
        }

        // Main pass: visibility buffer, then shade each pixel once.
        let mut _p = profile::scope("render.main");
        let pass = Pass {
            view_proj,
            w,
            h,
            px_scale: h as f32 * 0.5 / (cam.fov_y * 0.5).tan(),
            cull: Cull::Back,
            attributes: true,
        };
        let (verts, tris) = geometry(jobs, arena, meshes, &snap.instances, &pass);
        bin(jobs, bins, tris, h, settings.strip_height);
        // Color, depth and visibility buffers, plus vertices/triangles read by raster and shading.
        _p.set_bytes(w * h * 12 + size_of_val(verts) + size_of_val(tris));
        profile::counter("render arena used (KiB)", arena.used() as f64 / 1024.0);
        *stats = RenderStats {
            instances: snap.instances.len(),
            triangles: tris.len(),
            binned: bins.start.last().copied().unwrap_or(0) as usize,
        };
        let shade = Shading {
            verts,
            tris,
            instances: &snap.instances,
            shadow,
            shadow_size: s,
            shadow_vp,
            light: -sun_dir,
            sun: snap.sun,
            eye: cam.eye,
            settings: *settings,
        };
        let (bins, strip) = (&*bins, settings.strip_height);
        jobs.par_zip((&mut color[..], &mut depth[..], &mut vis[..]), w * strip, |offset, (color, depth, vis)| {
            let y0 = offset / w;
            let rows = color.len() / w;
            depth.fill(1.0);
            vis.fill(NO_TRI);
            let mut target = Visibility { depth, vis, tri: 0 };
            for t in strip_tris(bins, y0 / strip) {
                target.tri = t;
                raster(&tris[t as usize], verts, y0, rows, w, &mut target);
            }
            let vis = &*target.vis;
            for row in 0..rows {
                let y = y0 + row;
                let sky = pack(settings.sky_horizon.lerp(settings.sky_top, 1.0 - y as f32 / h as f32));
                let (vis_row, color_row) = (&vis[row * w..(row + 1) * w], &mut color[row * w..(row + 1) * w]);
                for x in (0..w).step_by(8) {
                    let n = (w - x).min(8);
                    let mut ids = [NO_TRI; 8];
                    ids[..n].copy_from_slice(&vis_row[x..x + n]);
                    if ids.iter().all(|&t| t == NO_TRI) {
                        color_row[x..x + n].fill(sky);
                    } else {
                        color_row[x..x + n].copy_from_slice(&shade.pixels(ids, x, y, sky)[..n]);
                    }
                }
            }
        });
    }
}

impl Renderer for SoftwareRenderer {
    type Snapshot = Snapshot;

    fn extract(world: &World, jobs: &Jobs, frame: &FrameInfo, out: &mut Snapshot) {
        out.frame = frame.frame;
        out.camera = world.try_res::<Camera>().map(|c| *c).unwrap_or_default();
        out.sun = world.try_res::<Sun>().map(|s| *s).unwrap_or_default();
        let tables: Vec<_> = world.query::<(Transform, Renderable)>().collect();
        out.instances.clear();
        out.instances.resize(tables.iter().map(|t| t.len()).sum(), Instance::default());
        let mut rest = &mut out.instances[..];
        for table in tables {
            let (dst, tail) = std::mem::take(&mut rest).split_at_mut(table.len());
            rest = tail;
            let transforms = table.read::<Transform>();
            let renderables = table.read::<Renderable>();
            jobs.par_zip((dst, &*transforms, &*renderables), 2048, |_, (dst, t, r)| {
                for i in 0..dst.len() {
                    dst[i] = Instance { mesh: r[i].mesh, pos: t[i].pos, rot: t[i].rot, scale: t[i].scale, color: r[i].color };
                }
            });
        }
    }

    fn render(&mut self, snapshot: &Snapshot, jobs: &Jobs) {
        self.render_frame(snapshot, jobs);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Cull {
    Back,
    Front,
}

/// Everything that differs between the shadow pass and the main pass.
struct Pass {
    view_proj: Mat4,
    w: usize,
    h: usize,
    /// Pixels per world unit at clip-space w = 1 (for LOD selection).
    px_scale: f32,
    cull: Cull,
    /// Fill world position and normal (needed for shading, not for depth-only passes).
    attributes: bool,
}

/// Planes (xyz = normal, w = offset) of the clip volume -w<=x<=w, -w<=y<=w, 0<=z<=w.
fn frustum_planes(m: &Mat4) -> [Vec4; 6] {
    let (r0, r1, r2, r3) = (m.row(0), m.row(1), m.row(2), m.row(3));
    [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r2, r3 - r2].map(|p| p * (1.0 / p.xyz().length()))
}

/// Culls instances, picks LODs, transforms vertices and assembles triangles into arena arrays.
fn geometry<'a>(
    jobs: &Jobs,
    arena: &'a FrameArena,
    meshes: &[MeshEntry],
    instances: &[Instance],
    pass: &Pass,
) -> (&'a mut [Vert], &'a mut [Tri]) {
    let mut _p = profile::scope("render.geometry");

    let chosen = arena.alloc_slice(instances.len(), NO_MESH);
    let clips = arena.alloc_slice(instances.len(), false);
    let planes = frustum_planes(&pass.view_proj);
    jobs.par_zip((&mut *chosen, &mut *clips, instances), 512, |_, (chosen, clips, instances)| {
        for ((out, clip), inst) in chosen.iter_mut().zip(clips.iter_mut()).zip(instances) {
            let entry = &meshes[inst.mesh.0 as usize];
            let s = inst.scale;
            let r = entry.radius * s.x.abs().max(s.y.abs()).max(s.z.abs());
            let c = inst.pos;
            if planes.iter().any(|p| p.x * c.x + p.y * c.y + p.z * c.z + p.w < -r) {
                *out = NO_MESH;
                continue;
            }
            // Bounding sphere crosses the near plane: some triangles may need clipping. Only the
            // perspective pass clips (it has the attributes to interpolate).
            let near = planes[4];
            *clip = pass.attributes && near.x * c.x + near.y * c.y + near.z * c.z + near.w < r;
            let w = pass.view_proj.mul_vec4(c.extend(1.0)).w;
            let r_px = if w > 1e-5 { r * pass.px_scale / w } else { f32::INFINITY };
            *out = match entry.lods.iter().find(|(min_px, _)| r_px >= *min_px) {
                Some(&(_, id)) => id,
                None => entry.lods.last().map_or(inst.mesh.0, |l| l.1),
            };
        }
    });

    // Room per instance: its mesh, plus, if it may be clipped, up to 2 new vertices and 1 extra
    // triangle per triangle (a triangle cut by a plane becomes a triangle or a quad).
    let sizes = |m: u32, clip: bool| -> (usize, usize) {
        if m == NO_MESH {
            return (0, 0);
        }
        let mesh = &meshes[m as usize].mesh;
        let (nv, nt) = (mesh.positions.len(), mesh.triangle_count());
        if clip { (nv + 2 * nt, 2 * nt) } else { (nv, nt) }
    };
    let (mut nv, mut nt) = (0, 0);
    for (&m, &clip) in chosen.iter().zip(clips.iter()) {
        let (v, t) = sizes(m, clip);
        (nv, nt) = (nv + v, nt + t);
    }
    let verts = arena.alloc_slice(nv, Vert::default());
    let tris = arena.alloc_slice(nt, Tri { inst: CULLED, ..Tri::default() });
    // Instances in, mesh data read (24 B per vertex, 12 per triangle), vertices and triangles out.
    _p.set_bytes(size_of_val(instances) + nv * (24 + size_of::<Vert>()) + nt * (12 + size_of::<Tri>()));

    // Split the big arrays into one (verts, tris, first vertex index, mesh, clip) range per instance.
    let mut parts = Vec::with_capacity(instances.len());
    let (mut vrest, mut trest, mut base) = (&mut verts[..], &mut tris[..], 0u32);
    for (&m, &clip) in chosen.iter().zip(clips.iter()) {
        let (nv, nt) = sizes(m, clip);
        let (v, vtail) = std::mem::take(&mut vrest).split_at_mut(nv);
        let (t, ttail) = std::mem::take(&mut trest).split_at_mut(nt);
        parts.push((v, t, base, m, clip));
        (vrest, trest) = (vtail, ttail);
        base += nv as u32;
    }

    let (wf, hf) = (pass.w as f32, pass.h as f32);
    jobs.par_zip((&mut parts[..], instances), 16, |first, (parts, instances)| {
        for (k, ((verts, tris, base, m, clip), inst)) in parts.iter_mut().zip(instances).enumerate() {
            if *m == NO_MESH {
                continue;
            }
            let id = (first + k) as u32;
            let mesh = &meshes[*m as usize].mesh;
            for (out, (&p, &n)) in verts.iter_mut().zip(mesh.positions.iter().zip(&mesh.normals)) {
                let world = inst.rot.rotate(p * inst.scale) + inst.pos;
                let normal = if pass.attributes { inst.rot.rotate(n / inst.scale).normalize() } else { Vec3::ZERO };
                *out = project(world, normal, pass, wf, hf);
            }
            if !*clip {
                for (t, idx) in tris.iter_mut().zip(mesh.indices.chunks_exact(3)) {
                    *t = setup_triangle(verts, idx, *base, id, pass);
                }
                continue;
            }
            let (mut next_vert, mut next_tri) = (mesh.positions.len(), mesh.triangle_count());
            for (k, idx) in mesh.indices.chunks_exact(3).enumerate() {
                let (poly, n) = clip_near(verts, idx, &mut next_vert, pass, wf, hf);
                if n >= 3 {
                    tris[k] = setup_triangle(verts, &poly[..3], *base, id, pass);
                }
                if n == 4 {
                    tris[next_tri] = setup_triangle(verts, &[poly[0], poly[2], poly[3]], *base, id, pass);
                    next_tri += 1;
                }
            }
        }
    });
    (verts, tris)
}

/// World-space vertex to screen. Vertices behind the eye get `inv_w = 0`; setup drops triangles
/// that use one, so those must be clipped first.
fn project(world: Vec3, n: Vec3, pass: &Pass, wf: f32, hf: f32) -> Vert {
    let clip = pass.view_proj.mul_vec4(world.extend(1.0));
    let mut out = Vert::default();
    if clip.w > 1e-5 {
        let inv_w = 1.0 / clip.w;
        out.inv_w = inv_w;
        out.x = (clip.x * inv_w * 0.5 + 0.5) * wf;
        out.y = (0.5 - clip.y * inv_w * 0.5) * hf;
        out.z = clip.z * inv_w;
    }
    if pass.attributes {
        out.wpos = world;
        out.n = n;
    }
    out
}

/// Clips triangle `idx` against the near plane (clip-space z >= 0), Sutherland-Hodgman style.
/// New vertices are interpolated in world space and written at `verts[*next..]`. Returns the
/// polygon (indices into `verts`, in the original winding) and its vertex count: 0, 3 or 4.
fn clip_near(verts: &mut [Vert], idx: &[u32], next: &mut usize, pass: &Pass, wf: f32, hf: f32) -> ([u32; 4], usize) {
    let row = pass.view_proj.row(2);
    let dist = |v: &Vert| row.x * v.wpos.x + row.y * v.wpos.y + row.z * v.wpos.z + row.w;
    let d = [dist(&verts[idx[0] as usize]), dist(&verts[idx[1] as usize]), dist(&verts[idx[2] as usize])];
    if d.iter().all(|&d| d >= 0.0) {
        return ([idx[0], idx[1], idx[2], 0], 3);
    }
    let (mut poly, mut n) = ([0u32; 4], 0);
    for e in 0..3 {
        let (a, b) = (idx[e] as usize, idx[(e + 1) % 3] as usize);
        let (da, db) = (d[e], d[(e + 1) % 3]);
        if da >= 0.0 {
            poly[n] = a as u32;
            n += 1;
        }
        if (da >= 0.0) != (db >= 0.0) {
            let t = da / (da - db);
            let (va, vb) = (verts[a], verts[b]);
            let mut v = project(va.wpos.lerp(vb.wpos, t), va.n.lerp(vb.n, t).normalize(), pass, wf, hf);
            v.z = v.z.max(0.0); // on the plane, up to rounding
            verts[*next] = v;
            poly[n] = *next as u32;
            n += 1;
            *next += 1;
        }
    }
    (poly, n)
}

fn edge(a: &Vert, b: &Vert, px: f32, py: f32) -> f32 {
    (b.x - a.x) * (py - a.y) - (b.y - a.y) * (px - a.x)
}

fn setup_triangle(verts: &[Vert], idx: &[u32], base: u32, inst: u32, pass: &Pass) -> Tri {
    let (w, h) = (pass.w, pass.h);
    let culled = Tri { inst: CULLED, ..Tri::default() };
    let (mut i0, mut i1, mut i2) = (idx[0] as usize, idx[1] as usize, idx[2] as usize);
    let (a, b, c) = (&verts[i0], &verts[i1], &verts[i2]);
    // Behind the camera or crossing the near plane: dropped. The main pass clips first
    // (`clip_near`); the shadow pass is orthographic and sized to contain its casters.
    if a.inv_w == 0.0 || b.inv_w == 0.0 || c.inv_w == 0.0 || a.z < 0.0 || b.z < 0.0 || c.z < 0.0 {
        return culled;
    }
    if a.z > 1.0 && b.z > 1.0 && c.z > 1.0 {
        return culled;
    }
    // y points down on screen, so counter-clockwise (front-facing) triangles have negative area.
    let area = edge(a, b, c.x, c.y);
    let front = area < 0.0;
    if area == 0.0 || (pass.cull == Cull::Back && !front) || (pass.cull == Cull::Front && front) {
        return culled;
    }
    if front {
        std::mem::swap(&mut i1, &mut i2);
    }
    let (min_x, max_x) = (a.x.min(b.x).min(c.x), a.x.max(b.x).max(c.x));
    let (min_y, max_y) = (a.y.min(b.y).min(c.y), a.y.max(b.y).max(c.y));
    // Pixel centers at +0.5: first/last pixel whose center can be inside.
    let x0 = (min_x - 0.5).ceil().max(0.0);
    let x1 = (max_x - 0.5).floor().min(w as f32 - 1.0);
    let y0 = (min_y - 0.5).ceil().max(0.0);
    let y1 = (max_y - 0.5).floor().min(h as f32 - 1.0);
    if x0 > x1 || y0 > y1 {
        return culled; // too small to cover a pixel center, or off screen
    }
    i0 += base as usize;
    i1 += base as usize;
    i2 += base as usize;
    Tri { v: [i0 as u32, i1 as u32, i2 as u32], inst, min_x: x0 as u16, max_x: x1 as u16, min_y: y0 as u16, max_y: y1 as u16 }
}

/// Parallel counting sort of triangles into strips: count per block, prefix sum, scatter.
fn bin(jobs: &Jobs, bins: &mut Bins, tris: &[Tri], height: usize, strip: usize) {
    let _p = profile::scope_bytes("render.bin", tris.len() * (size_of::<Tri>() + 4));
    let strips = height.div_ceil(strip);
    let block = tris.len().div_ceil(BIN_BLOCKS).max(1);
    let blocks = tris.len().div_ceil(block).max(1);
    let block_tris = |b: usize| &tris[(b * block).min(tris.len())..((b + 1) * block).min(tris.len())];
    let strips_of = |t: &Tri| t.min_y as usize / strip..=t.max_y as usize / strip;

    bins.counts.clear();
    bins.counts.resize(blocks * strips, 0);
    jobs.par_zip(&mut bins.counts[..], strips, |offset, row| {
        for t in block_tris(offset / strips).iter().filter(|t| t.inst != CULLED) {
            for s in strips_of(t) {
                row[s] += 1;
            }
        }
    });
    // Strip-major prefix sum: inside a strip, block 0's triangles come first, then block 1's...
    // so every strip lists its triangles in index order regardless of how work was split.
    bins.start.clear();
    let mut sum = 0u32;
    for s in 0..strips {
        bins.start.push(sum);
        for b in 0..blocks {
            let slot = &mut bins.counts[b * strips + s];
            let n = *slot;
            *slot = sum;
            sum += n;
        }
    }
    bins.start.push(sum);
    if bins.items.len() < sum as usize {
        bins.items.resize_with(sum as usize, || AtomicU32::new(0));
    }
    let items = &bins.items;
    jobs.par_zip(&mut bins.counts[..], strips, |offset, cursor| {
        let b = offset / strips;
        for (k, t) in block_tris(b).iter().enumerate() {
            if t.inst == CULLED {
                continue;
            }
            for s in strips_of(t) {
                items[cursor[s] as usize].store((b * block + k) as u32, Ordering::Relaxed);
                cursor[s] += 1;
            }
        }
    });
}

/// Triangle ids of a strip, in submission order.
fn strip_tris(bins: &Bins, strip: usize) -> impl Iterator<Item = u32> + '_ {
    let (a, b) = (bins.start[strip] as usize, bins.start[strip + 1] as usize);
    bins.items[a..b].iter().map(|i| i.load(Ordering::Relaxed))
}

/// Receives rasterized fragments. `test8` gets 8 consecutive pixels of one row.
trait DepthTarget {
    fn test8(&mut self, idx: usize, z: F32x8, covered: M32x8);
    fn test1(&mut self, idx: usize, z: f32);
}

/// Shadow map: keep the nearest depth.
struct DepthOnly<'a> {
    depth: &'a mut [f32],
}

impl DepthTarget for DepthOnly<'_> {
    #[inline(always)]
    fn test8(&mut self, idx: usize, z: F32x8, covered: M32x8) {
        let d = &mut self.depth[idx..idx + 8];
        let old = F32x8::load(d);
        (covered & z.lt(old)).select(z, old).store(d);
    }
    #[inline(always)]
    fn test1(&mut self, idx: usize, z: f32) {
        if z < self.depth[idx] {
            self.depth[idx] = z;
        }
    }
}

/// Visibility buffer: nearest depth plus the id of the triangle that produced it.
struct Visibility<'a> {
    depth: &'a mut [f32],
    vis: &'a mut [u32],
    tri: u32,
}

impl DepthTarget for Visibility<'_> {
    #[inline(always)]
    fn test8(&mut self, idx: usize, z: F32x8, covered: M32x8) {
        let d = &mut self.depth[idx..idx + 8];
        let old = F32x8::load(d);
        let pass = covered & z.lt(old);
        if pass.any() {
            pass.select(z, old).store(d);
            let ids = &mut self.vis[idx..idx + 8];
            pass.select_i32(I32x8::splat(self.tri as i32), I32x8::load_u32(ids)).store_u32(ids);
        }
    }
    #[inline(always)]
    fn test1(&mut self, idx: usize, z: f32) {
        if z < self.depth[idx] {
            self.depth[idx] = z;
            self.vis[idx] = self.tri;
        }
    }
}

/// Rasterizes `tri` inside rows `y0..y0+rows` of a strip `w` pixels wide, 8 pixels per step.
/// Blocks are aligned to multiples of 8 so neighbouring triangles evaluate the same lanes.
#[inline]
fn raster(tri: &Tri, verts: &[Vert], y0: usize, rows: usize, w: usize, target: &mut impl DepthTarget) {
    let (a, b, c) = (&verts[tri.v[0] as usize], &verts[tri.v[1] as usize], &verts[tri.v[2] as usize]);
    let ya = (tri.min_y as usize).max(y0);
    let yb = (tri.max_y as usize).min(y0 + rows - 1);
    if ya > yb {
        return;
    }
    let inv_area = 1.0 / edge(a, b, c.x, c.y);
    // An edge function grows by -(q.y - p.y) per pixel to the right; lane k is k pixels right.
    let lane = F32x8::ramp(0.0);
    let (d0, d1, d2) = (lane * F32x8::splat(b.y - c.y), lane * F32x8::splat(c.y - a.y), lane * F32x8::splat(a.y - b.y));
    let (za, zb, zc) = (F32x8::splat(a.z * inv_area), F32x8::splat(b.z * inv_area), F32x8::splat(c.z * inv_area));
    let (min_x, max_x) = (tri.min_x as usize, tri.max_x as usize);
    let (lo, hi) = (F32x8::splat(min_x as f32), F32x8::splat(max_x as f32));
    let zero = F32x8::splat(0.0);
    for y in ya..=yb {
        let py = y as f32 + 0.5;
        let row = (y - y0) * w;
        for bx in (min_x & !7..=max_x).step_by(8) {
            let px = bx as f32 + 0.5;
            let w0 = F32x8::splat(edge(b, c, px, py)) + d0;
            let w1 = F32x8::splat(edge(c, a, px, py)) + d1;
            let w2 = F32x8::splat(edge(a, b, px, py)) + d2;
            let x = F32x8::ramp(bx as f32);
            let covered = w0.ge(zero) & w1.ge(zero) & w2.ge(zero) & x.ge(lo) & x.le(hi);
            if !covered.any() {
                continue;
            }
            let z = w0.mul_add(za, w1.mul_add(zb, w2 * zc));
            if bx + 8 <= w {
                target.test8(row + bx, z, covered);
            } else {
                // Last block of a row whose width isn't a multiple of 8.
                let (z, bits) = (z.to_array(), covered.bits());
                for k in (0..8).filter(|k| bits >> k & 1 != 0) {
                    target.test1(row + bx + k, z[k]);
                }
            }
        }
    }
}

struct Shading<'a> {
    verts: &'a [Vert],
    tris: &'a [Tri],
    instances: &'a [Instance],
    shadow: &'a [f32],
    shadow_size: usize,
    shadow_vp: Mat4,
    light: Vec3,
    sun: Sun,
    eye: Vec3,
    settings: RenderSettings,
}

#[inline(always)]
fn splat3(v: Vec3) -> (F32x8, F32x8, F32x8) {
    (F32x8::splat(v.x), F32x8::splat(v.y), F32x8::splat(v.z))
}

#[inline(always)]
fn dot3(a: (F32x8, F32x8, F32x8), b: (F32x8, F32x8, F32x8)) -> F32x8 {
    a.0.mul_add(b.0, a.1.mul_add(b.1, a.2 * b.2))
}

#[inline(always)]
fn normalize3(v: (F32x8, F32x8, F32x8)) -> (F32x8, F32x8, F32x8) {
    let inv = F32x8::splat(1.0) / dot3(v, v).sqrt();
    (v.0 * inv, v.1 * inv, v.2 * inv)
}

impl Shading<'_> {
    /// Shades 8 pixels of row `y` starting at column `x`. `ids` are visibility-buffer entries;
    /// lanes without a triangle get `sky`.
    fn pixels(&self, ids: [u32; 8], x: usize, y: usize, sky: u32) -> [u32; 8] {
        let (one, zero) = (F32x8::splat(1.0), F32x8::splat(0.0));
        // Empty lanes borrow a covered lane's triangle (always a live one), result discarded below.
        let live = ids.iter().copied().find(|&t| t != NO_TRI).expect("pixels() needs a covered lane");
        let ids = I32x8::from_array(ids.map(|t| t as i32));
        let covered = !ids.eq(I32x8::splat(NO_TRI as i32));
        let t = covered.select_i32(ids, I32x8::splat(live as i32)) * I32x8::splat(Tri::WORDS as i32);
        let tw = as_u32(self.tris);
        let word = |k: i32| I32x8::gather_u32(tw, t + I32x8::splat(k));
        let (v0, v1, v2, inst) = (word(0), word(1), word(2), word(3));

        let vf = as_f32(self.verts);
        let attr = |v: I32x8, k: i32| F32x8::gather(vf, v * I32x8::splat(Vert::WORDS as i32) + I32x8::splat(k));
        let (ax, ay, aw) = (attr(v0, 0), attr(v0, 1), attr(v0, 3));
        let (bx, by, bw) = (attr(v1, 0), attr(v1, 1), attr(v1, 3));
        let (cx, cy, cw) = (attr(v2, 0), attr(v2, 1), attr(v2, 3));

        // Screen-space barycentrics, then perspective-correct weights via 1/w.
        let (px, py) = (F32x8::ramp(x as f32 + 0.5), F32x8::splat(y as f32 + 0.5));
        let edge = |x0: F32x8, y0: F32x8, x1: F32x8, y1: F32x8| (x1 - x0) * (py - y0) - (y1 - y0) * (px - x0);
        let inv_area = one / ((bx - ax) * (cy - ay) - (by - ay) * (cx - ax));
        let l0 = edge(bx, by, cx, cy) * inv_area * aw;
        let l1 = edge(cx, cy, ax, ay) * inv_area * bw;
        let l2 = edge(ax, ay, bx, by) * inv_area * cw;
        let norm = one / (l0 + l1 + l2);
        let (l0, l1, l2) = (l0 * norm, l1 * norm, l2 * norm);
        let interp = |k: i32| attr(v0, k).mul_add(l0, attr(v1, k).mul_add(l1, attr(v2, k) * l2));
        let wpos = (interp(4), interp(5), interp(6));
        let n = normalize3((interp(7), interp(8), interp(9)));

        let inf = as_f32(self.instances);
        let base = inst * I32x8::splat(Instance::WORDS as i32) + I32x8::splat(INSTANCE_COLOR);
        let albedo = [0, 1, 2].map(|k| F32x8::gather(inf, base + I32x8::splat(k)));

        let s = &self.settings;
        let light = splat3(self.light);
        let ndl = dot3(n, light).max(zero);
        let lit = ndl.gt(zero).select(self.shadow8(wpos, ndl), zero);
        let eye = splat3(self.eye);
        let to_eye = (eye.0 - wpos.0, eye.1 - wpos.1, eye.2 - wpos.2);
        let dist = dot3(to_eye, to_eye).sqrt();
        let inv_dist = one / dist;
        let view = (to_eye.0 * inv_dist, to_eye.1 * inv_dist, to_eye.2 * inv_dist);
        let half = normalize3((light.0 + view.0, light.1 + view.1, light.2 + view.2));
        let spec = dot3(n, half).max(zero).powi(s.shininess as u32) * F32x8::splat(s.specular) * lit;
        let hemi_t = n.1.mul_add(F32x8::splat(0.5), F32x8::splat(0.5));
        let direct = ndl * lit;
        let fog = one - (-dist * F32x8::splat(s.fog_density)).exp();
        let sun = [self.sun.color.x, self.sun.color.y, self.sun.color.z];
        let (ground, top, horizon) = (s.ground_bounce, s.sky_top, s.sky_horizon);
        let channel = |k: usize, g: f32, t: f32, hz: f32| {
            let hemi = F32x8::lerp(F32x8::splat(g), F32x8::splat(t), hemi_t) * F32x8::splat(self.sun.ambient);
            let sun_k = F32x8::splat(sun[k]);
            let lit_color = albedo[k] * sun_k.mul_add(direct, hemi) + sun_k * spec;
            F32x8::lerp(lit_color, F32x8::splat(hz), fog)
        };
        let r = channel(0, ground.x, top.x, horizon.x);
        let g = channel(1, ground.y, top.y, horizon.y);
        let b = channel(2, ground.z, top.z, horizon.z);
        covered.select_i32(pack8(r, g, b), I32x8::splat(sky as i32)).to_array().map(|c| c as u32)
    }

    /// 3x3 percentage-closer filtering for 8 world positions. 1 = fully lit.
    #[inline(always)]
    fn shadow8(&self, wpos: (F32x8, F32x8, F32x8), ndl: F32x8) -> F32x8 {
        let (one, zero) = (F32x8::splat(1.0), F32x8::splat(0.0));
        let m = &self.shadow_vp;
        // Orthographic: w == 1, so only the first three rows matter.
        let row = |r: usize| {
            let v = m.row(r);
            wpos.0.mul_add(F32x8::splat(v.x), wpos.1.mul_add(F32x8::splat(v.y), wpos.2.mul_add(F32x8::splat(v.z), F32x8::splat(v.w))))
        };
        let (cx, cy, cz) = (row(0), row(1), row(2));
        let size = self.shadow_size as f32;
        let half = F32x8::splat(0.5 * size);
        let sx = cx.mul_add(half, half - F32x8::splat(0.5));
        let sy = (-cy).mul_add(half, half - F32x8::splat(0.5));
        let limit = F32x8::splat(size - 1.0);
        let inside = sx.ge(zero) & sx.lt(limit) & sy.ge(zero) & sy.lt(limit) & cz.le(one);
        let z = cz - (one - ndl).mul_add(F32x8::splat(0.003), F32x8::splat(0.0015));
        let (ix, iy) = (sx.round().to_i32(), sy.round().to_i32());
        let (lo, hi, stride) = (I32x8::splat(0), I32x8::splat(self.shadow_size as i32 - 1), I32x8::splat(self.shadow_size as i32));
        let mut lit = zero;
        for dy in -1..=1 {
            let row = (iy + I32x8::splat(dy)).max(lo).min(hi) * stride;
            for dx in -1..=1 {
                let col = (ix + I32x8::splat(dx)).max(lo).min(hi);
                let depth = F32x8::gather(self.shadow, row + col);
                lit = lit + z.le(depth).select(one, zero);
            }
        }
        inside.select(lit * F32x8::splat(1.0 / 9.0), one)
    }
}

/// `pack` for 8 pixels.
#[inline(always)]
fn pack8(r: F32x8, g: F32x8, b: F32x8) -> I32x8 {
    let ch = |v: F32x8| {
        let v = v.max(F32x8::splat(0.0));
        let v = (v / v.mul_add(F32x8::splat(0.25), F32x8::splat(1.0))).sqrt();
        v.min(F32x8::splat(1.0)).mul_add(F32x8::splat(255.0), F32x8::splat(0.5)).to_i32()
    };
    ch(r).shl::<16>() | ch(g).shl::<8>() | ch(b)
}

/// Linear color -> 0x00RRGGBB with Reinhard-ish shoulder and a cheap gamma (sqrt).
fn pack(c: Vec3) -> u32 {
    let ch = |v: f32| {
        let v = v.max(0.0);
        let v = (v / (1.0 + 0.25 * v)).sqrt();
        (v.min(1.0) * 255.0 + 0.5) as u32
    };
    (ch(c.x) << 16) | (ch(c.y) << 8) | ch(c.z)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::JobSystem;

    fn assert_outward(m: &Mesh) {
        for t in m.indices.chunks_exact(3) {
            let (a, b, c) = (m.positions[t[0] as usize], m.positions[t[1] as usize], m.positions[t[2] as usize]);
            let centroid = (a + b + c) / 3.0;
            assert!((b - a).cross(c - a).dot(centroid) > 0.0, "inward-facing triangle {t:?}");
        }
    }

    #[test]
    fn meshes_wind_counter_clockwise_from_outside() {
        assert_outward(&Mesh::sphere(12, 8));
        assert_outward(&Mesh::cube());
        assert_outward(&Mesh::cylinder(16));
    }

    fn scene() -> Snapshot {
        let mut snap = Snapshot::default();
        snap.camera = Camera { eye: vec3(0.0, 2.0, 6.0), target: vec3(0.0, 1.0, 0.0), ..Camera::default() };
        snap.sun.shadow_radius = 6.0;
        snap.instances.push(Instance { mesh: MeshId(1), pos: vec3(0.0, -0.5, 0.0), scale: vec3(5.0, 0.5, 5.0), color: Vec3::splat(0.6), ..Instance::default() });
        snap.instances.push(Instance { mesh: MeshId(0), pos: vec3(0.0, 1.0, 0.0), scale: Vec3::ONE, color: vec3(0.8, 0.2, 0.1), ..Instance::default() });
        snap
    }

    fn renderer() -> SoftwareRenderer {
        let mut r = SoftwareRenderer::new(160, 120);
        r.settings.shadow_size = 256;
        r.add_mesh(Mesh::sphere(16, 10));
        r.add_mesh(Mesh::cube());
        r
    }

    #[test]
    fn draws_the_sphere_in_the_middle_and_sky_in_the_corner() {
        let jobs = JobSystem::new(3);
        let mut r = renderer();
        r.render(&scene(), &jobs);
        let center = r.color[60 * 160 + 80];
        let red = (center >> 16) & 0xff;
        let blue = center & 0xff;
        assert!(red > blue + 40, "center pixel {center:06x} should be the red sphere");
        let corner = r.color[0];
        assert!(corner & 0xff > (corner >> 16) & 0xff, "top-left {corner:06x} should be sky blue");
        assert!(r.stats.binned > 0);
    }

    #[test]
    fn sphere_casts_a_shadow_on_the_ground() {
        let jobs = JobSystem::new(3);
        let mut snap = scene();
        snap.sun.dir = vec3(0.0, -1.0, 0.0);
        snap.camera = Camera { eye: vec3(0.0, 8.0, 0.01), target: Vec3::ZERO, ..Camera::default() };
        snap.instances[1].pos = vec3(2.5, 1.2, 0.0);
        let mut r = renderer();
        r.render(&snap, &jobs);
        // Looking straight down from x=0, ground point x=1.6 is under the sphere's shadow
        // (x in 1.5..3.5) but still visible past its silhouette. ~13.7 px per world unit.
        let lum = |p: u32| ((p >> 16) & 0xff) + ((p >> 8) & 0xff) + (p & 0xff);
        let mut r2 = renderer();
        snap.instances[1].pos = vec3(2.5, 1.2, 50.0); // sphere moved away: no shadow there
        r2.render(&snap, &jobs);
        let spot = 60 * 160 + 80 + 22;
        assert!(lum(r.color[spot]) < lum(r2.color[spot]), "expected shadow: {:06x} vs {:06x}", r.color[spot], r2.color[spot]);
    }

    #[test]
    fn image_does_not_depend_on_thread_count() {
        let mut images = Vec::new();
        for workers in [0, 2, 7] {
            let jobs = JobSystem::new(workers);
            let mut r = renderer();
            r.render(&scene(), &jobs);
            images.push(r.color);
        }
        assert!(images.windows(2).all(|i| i[0] == i[1]));
    }
}
