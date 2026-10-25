//! Draws the 3D viewport into an offscreen texture that egui shows as an
//! image. Rendering only happens when something changed (camera, model,
//! theme, size), so an idle window costs nothing.

use crate::mesh::SceneMesh;
use bytemuck::{Pod, Zeroable};
use glam::Mat4;
use wgpu::util::DeviceExt;

const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DISPLAY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// 4x MSAA is guaranteed for these formats on every WebGPU backend.
const SAMPLES: u32 = 4;

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Vertex {
    pos: [f32; 3],
    normal: [f32; 3],
    /// Index into the mesh's faces.
    face: u32,
    /// Highlight group: the bodies a feature created (see [`SceneMesh::group`]).
    group: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct LineVertex {
    pos: [f32; 3],
    group: u32,
}

/// Colors are linear RGB(A).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct Uniforms {
    pub view_proj: [[f32; 4]; 4],
    /// World-space direction towards the key light.
    pub light: [f32; 4],
    /// World-space direction towards the viewer.
    pub eye: [f32; 4],
    pub body: [f32; 4],
    pub edge: [f32; 4],
    pub highlight: [f32; 4],
    pub bg_top: [f32; 4],
    pub bg_bottom: [f32; 4],
    /// x: selected highlight group, y: selected face; `u32::MAX` for none.
    pub selected: [u32; 4],
}

impl Uniforms {
    pub fn new(view_proj: Mat4) -> Self {
        Uniforms {
            view_proj: view_proj.to_cols_array_2d(),
            light: [0.0, 0.0, 1.0, 0.0],
            eye: [0.0, 0.0, 1.0, 0.0],
            body: [0.6, 0.6, 0.6, 1.0],
            edge: [0.0, 0.0, 0.0, 1.0],
            highlight: [0.2, 0.4, 1.0, 1.0],
            bg_top: [0.1, 0.1, 0.1, 1.0],
            bg_bottom: [0.2, 0.2, 0.2, 1.0],
            selected: [u32::MAX, u32::MAX, 0, 0],
        }
    }
}

const SHADER: &str = r#"
struct U {
    view_proj: mat4x4<f32>,
    light: vec4<f32>,
    eye: vec4<f32>,
    body: vec4<f32>,
    edge: vec4<f32>,
    highlight: vec4<f32>,
    bg_top: vec4<f32>,
    bg_bottom: vec4<f32>,
    selected: vec4<u32>,
};
@group(0) @binding(0) var<uniform> u: U;

struct BgOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) t: f32,
};

@vertex
fn vs_bg(@builtin(vertex_index) i: u32) -> BgOut {
    // One oversized triangle covering the screen.
    let xy = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    var out: BgOut;
    out.pos = vec4<f32>(xy, 1.0, 1.0);
    out.t = xy.y * 0.5 + 0.5;
    return out;
}

@fragment
fn fs_bg(in: BgOut) -> @location(0) vec4<f32> {
    return mix(u.bg_bottom, u.bg_top, in.t);
}

struct MeshOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) @interpolate(flat) face: u32,
    @location(2) @interpolate(flat) group: u32,
};

@vertex
fn vs_mesh(
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) face: u32,
    @location(3) group: u32,
) -> MeshOut {
    var out: MeshOut;
    out.pos = u.view_proj * vec4<f32>(pos, 1.0);
    out.normal = normal;
    out.face = face;
    out.group = group;
    return out;
}

@fragment
fn fs_mesh(in: MeshOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    var n = normalize(in.normal);
    if (!front) { n = -n; }
    var base = u.body.rgb;
    // A picked face is fully highlighted; the rest of its feature's bodies
    // are tinted, like Inventor's selection.
    if (in.face == u.selected.y) {
        base = u.highlight.rgb;
    } else if (in.group == u.selected.x) {
        base = mix(base, u.highlight.rgb, 0.45);
    }
    let l = normalize(u.light.xyz);
    let v = normalize(u.eye.xyz);
    let diffuse = max(dot(n, l), 0.0);
    // Soft fill from the viewer so faces turned from the key light keep shape.
    let fill = max(dot(n, v), 0.0);
    let h = normalize(l + v);
    let spec = pow(max(dot(n, h), 0.0), 40.0) * 0.18;
    let color = base * (0.22 + 0.55 * diffuse + 0.25 * fill) + vec3<f32>(spec);
    return vec4<f32>(color, 1.0);
}

struct LineOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) @interpolate(flat) group: u32,
};

@vertex
fn vs_line(@location(0) pos: vec3<f32>, @location(1) group: u32) -> LineOut {
    var out: LineOut;
    out.pos = u.view_proj * vec4<f32>(pos, 1.0);
    // Pull edges slightly towards the camera so they win the depth test
    // against their own faces. Orthographic, so w is 1.
    out.pos.z = out.pos.z - 2e-4;
    out.group = group;
    return out;
}

@fragment
fn fs_line(in: LineOut) -> @location(0) vec4<f32> {
    if (in.group == u.selected.x) { return vec4<f32>(u.highlight.rgb * 0.55, 1.0); }
    return u.edge;
}
"#;

struct Targets {
    size: [u32; 2],
    msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
    /// sRGB view the render pass resolves into (encodes on write).
    resolve: wgpu::TextureView,
    /// Plain view of the same texture for egui, which expects sRGB-encoded
    /// bytes and must not decode them again on sampling.
    display: wgpu::TextureView,
    #[cfg(test)]
    resolve_texture: wgpu::Texture,
}

pub struct ViewportRenderer {
    bg: wgpu::RenderPipeline,
    mesh: wgpu::RenderPipeline,
    lines: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// Vertex buffer, index buffer, index count.
    triangles: Option<(wgpu::Buffer, wgpu::Buffer, u32)>,
    edges: Option<(wgpu::Buffer, u32)>,
    targets: Option<Targets>,
    texture: Option<egui::TextureId>,
}

impl ViewportRenderer {
    pub fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("viewport"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewport uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewport"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewport"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewport"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = |label: &str,
                        vs: &str,
                        fs: &str,
                        buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
                        topology: wgpu::PrimitiveTopology,
                        depth_write: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers,
                },
                primitive: wgpu::PrimitiveState {
                    topology,
                    // Two-sided: open or inside-out geometry still shows.
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: SAMPLES,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(COLOR_FORMAT.into())],
                }),
                multiview_mask: None,
                cache: None,
            })
        };

        let mesh_attrs =
            wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Uint32, 3 => Uint32];
        let line_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Uint32];
        let mesh_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &mesh_attrs,
        };
        let line_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<LineVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &line_attrs,
        };
        ViewportRenderer {
            bg: pipeline(
                "viewport background",
                "vs_bg",
                "fs_bg",
                &[],
                wgpu::PrimitiveTopology::TriangleList,
                false,
            ),
            mesh: pipeline(
                "viewport mesh",
                "vs_mesh",
                "fs_mesh",
                &[Some(mesh_layout)],
                wgpu::PrimitiveTopology::TriangleList,
                true,
            ),
            lines: pipeline(
                "viewport edges",
                "vs_line",
                "fs_line",
                &[Some(line_layout)],
                wgpu::PrimitiveTopology::LineList,
                true,
            ),
            uniforms,
            bind_group,
            triangles: None,
            edges: None,
            targets: None,
            texture: None,
        }
    }

    pub fn set_mesh(&mut self, device: &wgpu::Device, scene: &SceneMesh) {
        let mesh = &scene.mesh;
        let upload = |label: &str, data: &[u8], usage| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: data,
                usage,
            })
        };
        let owners = mesh.vertex_faces();
        let vertices: Vec<Vertex> = (0..mesh.positions.len())
            .map(|i| {
                let face = owners[i];
                let body = mesh.faces.get(face as usize).map_or(0, |f| f.body);
                Vertex {
                    pos: mesh.positions[i],
                    normal: mesh.normals[i],
                    face,
                    group: scene.group(body),
                }
            })
            .collect();
        self.triangles = (!mesh.indices.is_empty()).then(|| {
            (
                upload(
                    "viewport vertices",
                    bytemuck::cast_slice(&vertices),
                    wgpu::BufferUsages::VERTEX,
                ),
                upload(
                    "viewport indices",
                    bytemuck::cast_slice(&mesh.indices),
                    wgpu::BufferUsages::INDEX,
                ),
                mesh.indices.len() as u32,
            )
        });
        // Polylines become segment pairs for a line list.
        let mut lines = Vec::new();
        for run in &mesh.edges {
            let group = scene.group(run.body);
            let first = run.first_point as usize;
            let points = &mesh.edge_points[first..first + run.point_count as usize];
            for pair in points.windows(2) {
                lines.extend(pair.iter().map(|&pos| LineVertex { pos, group }));
            }
        }
        self.edges = (!lines.is_empty()).then(|| {
            (
                upload(
                    "viewport edges",
                    bytemuck::cast_slice(&lines),
                    wgpu::BufferUsages::VERTEX,
                ),
                lines.len() as u32,
            )
        });
    }

    /// Renders a frame of `size` physical pixels and returns the egui texture
    /// holding it.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        egui_renderer: &mut egui_wgpu::Renderer,
        size: [u32; 2],
        uniforms: &Uniforms,
    ) -> egui::TextureId {
        let resized = self.draw(device, queue, size, uniforms);
        let view = &self.targets.as_ref().expect("drawn above").display;
        let filter = wgpu::FilterMode::Nearest;
        match self.texture {
            Some(id) if resized => {
                egui_renderer.update_egui_texture_from_wgpu_texture(device, view, filter, id)
            }
            Some(_) => {}
            None => {
                self.texture = Some(egui_renderer.register_native_texture(device, view, filter))
            }
        }
        self.texture.expect("registered above")
    }

    /// Draws into the offscreen target, recreating it at `size` if needed.
    /// Returns true if the target was recreated.
    fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: [u32; 2],
        uniforms: &Uniforms,
    ) -> bool {
        let limit = device.limits().max_texture_dimension_2d;
        let size = size.map(|s| s.clamp(1, limit));
        let resized = self.targets.as_ref().is_none_or(|t| t.size != size);
        if resized {
            self.targets = Some(make_targets(device, size));
        }
        let targets = self.targets.as_ref().expect("created above");
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(uniforms));

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("viewport"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.msaa,
                    depth_slice: None,
                    resolve_target: Some(&targets.resolve),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_pipeline(&self.bg);
            pass.draw(0..3, 0..1);
            if let Some((vertices, indices, count)) = &self.triangles {
                pass.set_pipeline(&self.mesh);
                pass.set_vertex_buffer(0, vertices.slice(..));
                pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..*count, 0, 0..1);
            }
            if let Some((buffer, count)) = &self.edges {
                pass.set_pipeline(&self.lines);
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw(0..*count, 0..1);
            }
        }
        queue.submit([encoder.finish()]);
        resized
    }
}

fn make_targets(device: &wgpu::Device, size: [u32; 2]) -> Targets {
    let texture = |label: &str, format, samples, usage, view_formats: &[wgpu::TextureFormat]| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats,
        })
    };
    let resolve = texture(
        "viewport color",
        COLOR_FORMAT,
        1,
        wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        &[DISPLAY_FORMAT],
    );
    let view = |t: wgpu::Texture| t.create_view(&Default::default());
    Targets {
        size,
        resolve: resolve.create_view(&Default::default()),
        display: resolve.create_view(&wgpu::TextureViewDescriptor {
            format: Some(DISPLAY_FORMAT),
            ..Default::default()
        }),
        #[cfg(test)]
        resolve_texture: resolve,
        msaa: view(texture(
            "viewport msaa",
            COLOR_FORMAT,
            SAMPLES,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
            &[],
        )),
        depth: view(texture(
            "viewport depth",
            DEPTH_FORMAT,
            SAMPLES,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
            &[],
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::{Camera, StandardView};
    use crate::demo;
    use crate::mesh::tessellate;

    /// A device on whatever adapter exists, software included. `None` on
    /// machines with no adapter at all, where the test is skipped.
    fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: false,
            compatible_surface: None,
            ..Default::default()
        }))
        .ok()?;
        pollster::block_on(adapter.request_device(&Default::default())).ok()
    }

    fn read(device: &wgpu::Device, queue: &wgpu::Queue, r: &ViewportRenderer) -> (u32, Vec<u8>) {
        let t = r.targets.as_ref().unwrap();
        let [w, h] = t.size;
        let stride = (w * 4).div_ceil(256) * 256;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (stride * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            t.resolve_texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = buffer.slice(..).get_mapped_range().unwrap().to_vec();
        (stride, data)
    }

    fn pixel((stride, data): &(u32, Vec<u8>), x: u32, y: u32) -> [u8; 3] {
        let i = (y * stride + x * 4) as usize;
        [data[i], data[i + 1], data[i + 2]]
    }

    #[test]
    fn draws_background_body_and_highlight() {
        let Some((device, queue)) = device() else {
            eprintln!("no GPU adapter; skipping");
            return;
        };
        let model = sm_kernel::rebuild(&demo::document()).unwrap();
        let scene = tessellate(&model, &Default::default()).unwrap();
        let mut camera = Camera::default();
        camera.scene_center = scene.center;
        camera.scene_radius = scene.radius;
        camera.go_to(StandardView::Front, 0.0);
        camera.animate(1.0);
        camera.fit(1.0);
        let mut r = ViewportRenderer::new(&device);
        r.set_mesh(&device, &scene);

        let mut u = Uniforms::new(camera.projection(1.0) * camera.view());
        u.light = [0.0, 0.0, 1.0, 0.0];
        u.eye = [0.0, 0.0, 1.0, 0.0];
        u.bg_top = [0.0, 0.0, 0.0, 1.0];
        u.bg_bottom = [0.0, 0.0, 0.0, 1.0];
        u.body = [0.5, 0.5, 0.5, 1.0];
        u.highlight = [1.0, 0.0, 0.0, 1.0];
        assert!(r.draw(&device, &queue, [64, 64], &u));
        let frame = read(&device, &queue, &r);
        // Corner: background. Centre (inside the pocket floor): grey body.
        assert_eq!(pixel(&frame, 1, 1), [0, 0, 0]);
        let body = pixel(&frame, 32, 32);
        assert!(
            body[0] > 60 && body[0] == body[1] && body[1] == body[2],
            "{body:?}"
        );

        // Same size: no new target. The picked face turns red.
        let ray = camera.ray(glam::Vec2::splat(32.0), glam::Vec2::splat(64.0));
        let (face, _) = scene.mesh.pick(ray.0.into(), ray.1.into()).unwrap();
        u.selected[1] = face as u32;
        assert!(!r.draw(&device, &queue, [64, 64], &u));
        let frame = read(&device, &queue, &r);
        let lit = pixel(&frame, 32, 32);
        // Red plus a white specular highlight (light and eye coincide).
        assert!(lit[0] == 255 && lit[1] == lit[2] && lit[1] < 160, "{lit:?}");
        // Without a selected feature the other faces stay grey.
        let side = pixel(&frame, 12, 32);
        assert!(side[0] == side[1] && side[1] == side[2], "{side:?}");

        // A pick selects the face and its feature: the rest of the body is
        // only tinted.
        u.selected[0] = 0;
        r.draw(&device, &queue, [64, 64], &u);
        let frame = read(&device, &queue, &r);
        assert_eq!(pixel(&frame, 32, 32), lit);
        let side = pixel(&frame, 12, 32);
        assert!(side[0] > side[1] && side[1] > 40, "{side:?}");

        // Selecting the feature's group tints the whole body.
        u.selected = [0, u32::MAX, 0, 0];
        r.draw(&device, &queue, [64, 64], &u);
        let frame = read(&device, &queue, &r);
        let tinted = pixel(&frame, 32, 32);
        assert!(tinted[0] > tinted[1] && tinted[1] > 40, "{tinted:?}");

        // A zero-size request is clamped instead of failing validation.
        assert!(r.draw(&device, &queue, [0, 0], &u));
    }
}
