//! Instanced rectangle batch pipeline.

use bytemuck::{Pod, Zeroable};
use ui::scene::{RectInstance, Scene};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Screen {
    pub size: [f32; 2],
    pub scale: f32,
    pub srgb: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct GpuRect {
    pos: [f32; 2],
    size: [f32; 2],
    fill: [f32; 4],
    border: [f32; 4],
    params: [f32; 4],
}

impl From<&RectInstance> for GpuRect {
    fn from(r: &RectInstance) -> Self {
        Self {
            pos: [r.rect.x, r.rect.y],
            size: [r.rect.w, r.rect.h],
            fill: r.fill,
            border: r.border,
            params: [r.border_width, r.radius, r.glow, r.kind as u32 as f32],
        }
    }
}

pub struct RectPipeline {
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    instances: wgpu::Buffer,
    capacity: usize,
    count: u32,
    staging: Vec<GpuRect>,
}

impl RectPipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/rect.wgsl"));
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("edex rect screen uniform"),
            contents: bytemuck::bytes_of(&Screen {
                size: [1.0, 1.0],
                scale: 1.0,
                srgb: 0.0,
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("edex rect bgl"),
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
            label: Some("edex rect bg"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("edex rect pl"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuRect>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4],
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("edex rect pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[instance_layout],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let capacity = 1024;
        let instances = Self::make_buffer(device, capacity);
        Self {
            pipeline,
            uniform,
            bind_group,
            instances,
            capacity,
            count: 0,
            staging: Vec::new(),
        }
    }

    fn make_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("edex rect instances"),
            size: (capacity * std::mem::size_of::<GpuRect>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Upload this frame's rectangles; the instance buffer only grows.
    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &Scene,
        scale: f32,
        srgb: bool,
        background: bool,
    ) {
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&Screen {
                size: [scene.width, scene.height],
                scale,
                srgb: if srgb { 1.0 } else { 0.0 },
            }),
        );
        self.staging.clear();
        if background {
            // The scene's clear colour as a first, full-size rectangle (partial redraws
            // cannot clear the whole target).
            self.staging.push(GpuRect {
                pos: [0.0, 0.0],
                size: [scene.width, scene.height],
                fill: scene.clear,
                border: [0.0; 4],
                params: [0.0; 4],
            });
        }
        self.staging.extend(
            scene
                .rects
                .iter()
                .filter(|r| r.rect.w > 0.0 && r.rect.h > 0.0)
                .map(GpuRect::from),
        );
        if self.staging.len() > self.capacity {
            self.capacity = self.staging.len().next_power_of_two();
            self.instances = Self::make_buffer(device, self.capacity);
        }
        if !self.staging.is_empty() {
            queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&self.staging));
        }
        self.count = self.staging.len() as u32;
    }

    pub fn draw<'p>(&'p self, pass: &mut wgpu::RenderPass<'p>) {
        if self.count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        pass.draw(0..6, 0..self.count);
    }

    pub fn instance_capacity(&self) -> usize {
        self.capacity
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct ScanParams {
    pub size: [f32; 2],
    pub scale: f32,
    pub intensity: f32,
    pub color: [f32; 4],
}

pub struct ScanlinePipeline {
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl ScanlinePipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/scanline.wgsl"));
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("edex scanline uniform"),
            contents: bytemuck::bytes_of(&ScanParams {
                size: [1.0, 1.0],
                scale: 1.0,
                intensity: 0.0,
                color: [0.0; 4],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("edex scanline bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("edex scanline bg"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("edex scanline pl"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("edex scanline pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            uniform,
            bind_group,
        }
    }

    pub fn update(&self, queue: &wgpu::Queue, params: ScanParams) {
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&params));
    }

    pub fn draw<'p>(&'p self, pass: &mut wgpu::RenderPass<'p>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}
