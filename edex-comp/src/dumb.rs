//! Software output: pixman renders into mapped DRM dumb buffers, which are page-flipped on the
//! CRTC's primary plane. Needs nothing but KMS (no GBM, no EGL), so it runs on any DRM driver,
//! including simple ones like bochs or simpledrm.

use std::io;

use smithay::{
    backend::{
        allocator::{
            dumb::{DumbAllocator, DumbBuffer},
            Allocator, Fourcc, Modifier,
        },
        drm::{
            dumb::{framebuffer_from_dumb_buffer, DumbFramebuffer},
            DrmDeviceFd, DrmError, DrmSurface, PlaneConfig, PlaneState,
        },
        renderer::{
            damage::{Error as DamageError, OutputDamageTracker},
            element::{RenderElement, RenderElementStates},
            pixman::{PixmanError, PixmanRenderer},
            Bind, Color32F,
        },
    },
    output::Output,
    reexports::{
        drm::{buffer::Buffer as _, control::Device as _},
        pixman,
    },
    utils::{Rectangle, Transform},
};

/// Errors of the software output.
#[derive(Debug, thiserror::Error)]
pub enum DumbError {
    #[error("allocating a dumb buffer: {0}")]
    Allocate(io::Error),
    #[error("mapping a dumb buffer: {0}")]
    Map(io::Error),
    #[error("adding a framebuffer: {0}")]
    Framebuffer(String),
    #[error("creating the pixman image")]
    Image,
    #[error("rendering: {0:?}")]
    Render(DamageError<PixmanError>),
    #[error("binding the buffer: {0}")]
    Bind(PixmanError),
    #[error(transparent)]
    Drm(#[from] DrmError),
}

/// A mapped dumb buffer with its framebuffer and a pixman image over its memory. Fields drop in
/// order: the image, the mapping, the framebuffer, the buffer.
struct Slot {
    image: pixman::Image<'static, 'static>,
    _map: Mapping,
    fb: DumbFramebuffer,
    _buffer: DumbBuffer,
    /// Frames since this buffer was last shown (0: contents unknown).
    age: u8,
}

struct Mapping {
    ptr: *mut u8,
    len: usize,
}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: `ptr`/`len` come from a successful mmap that nothing else unmaps.
        unsafe {
            libc::munmap(self.ptr.cast(), self.len);
        }
    }
}

/// What a frame did.
pub struct DumbFrame {
    /// Whether anything changed and a page flip was queued.
    pub rendered: bool,
    pub states: RenderElementStates,
}

/// One CRTC driven by pixman and dumb buffers, double-buffered.
pub struct DumbOutput<U> {
    surface: DrmSurface,
    fd: DrmDeviceFd,
    allocator: DumbAllocator,
    slots: Vec<Slot>,
    size: (u32, u32),
    damage: OutputDamageTracker,
    /// The slot shown on screen.
    front: Option<usize>,
    /// A slot flipped to and waiting for its vblank, with the frame's user data.
    pending: Option<(usize, U)>,
    /// The slot rendered by `render_frame`, waiting for `queue_frame`.
    ready: Option<usize>,
    /// Buffers of the previous mode, kept until the next flip completes.
    retired: Vec<Slot>,
}

const FORMAT: Fourcc = Fourcc::Xrgb8888;

impl<U> DumbOutput<U> {
    pub fn new(surface: DrmSurface, fd: DrmDeviceFd, output: &Output) -> Self {
        let (w, h) = surface.pending_mode().size();
        DumbOutput {
            allocator: DumbAllocator::new(fd.clone()),
            surface,
            fd,
            slots: Vec::new(),
            size: (w as u32, h as u32),
            damage: OutputDamageTracker::from_output(output),
            front: None,
            pending: None,
            ready: None,
            retired: Vec::new(),
        }
    }

    pub fn surface(&self) -> &DrmSurface {
        &self.surface
    }

    pub fn format(&self) -> Fourcc {
        FORMAT
    }

    /// Forget the buffers' contents: the next frame is drawn in full.
    pub fn reset_buffers(&mut self) {
        for slot in &mut self.slots {
            slot.age = 0;
        }
    }

    /// Switch the CRTC to `mode` (applied with the next frame) and reallocate the buffers.
    pub fn use_mode(
        &mut self,
        mode: smithay::reexports::drm::control::Mode,
    ) -> Result<(), DumbError> {
        self.surface.use_mode(mode)?;
        let (w, h) = mode.size();
        self.size = (w as u32, h as u32);
        // The buffer on screen stays alive until the new mode's first frame replaces it:
        // removing a framebuffer that is being scanned out turns the CRTC off.
        self.retired.append(&mut self.slots);
        self.front = None;
        self.pending = None;
        self.ready = None;
        Ok(())
    }

    /// Turn the CRTC off; the next frame turns it on again.
    pub fn clear(&mut self) -> Result<(), DrmError> {
        self.pending = None;
        self.front = None;
        self.reset_buffers();
        self.surface.clear()
    }

    /// Re-read the CRTC state (after a VT switch) and redraw in full.
    pub fn reset_state(&mut self) -> Result<(), DrmError> {
        self.pending = None;
        self.reset_buffers();
        self.surface.reset_state()
    }

    fn new_slot(&mut self) -> Result<Slot, DumbError> {
        let (w, h) = self.size;
        let buffer = self
            .allocator
            .create_buffer(w, h, FORMAT, &[Modifier::Linear])
            .map_err(DumbError::Allocate)?;
        let fb = framebuffer_from_dumb_buffer(&self.fd, &buffer, false)
            .map_err(|e| DumbError::Framebuffer(e.to_string()))?;
        let mut handle = *buffer.handle();
        let pitch = handle.pitch() as usize;
        let mut mapping = self
            .fd
            .map_dumb_buffer(&mut handle)
            .map_err(DumbError::Map)?;
        let map = Mapping {
            ptr: mapping.as_mut().as_mut_ptr(),
            len: mapping.len(),
        };
        // `Mapping` unmaps it.
        std::mem::forget(mapping);
        // SAFETY: the memory stays mapped as long as the image (dropped before `map`).
        let image = unsafe {
            pixman::Image::from_raw_mut(
                pixman::FormatCode::X8R8G8B8,
                w as usize,
                h as usize,
                map.ptr.cast(),
                pitch,
                false,
            )
        }
        .map_err(|_| DumbError::Image)?;
        Ok(Slot {
            image,
            _map: map,
            fb,
            _buffer: buffer,
            age: 0,
        })
    }

    /// Render `elements` into a free buffer. Returns `rendered: false` when nothing changed or
    /// a flip is still in flight; otherwise [`queue_frame`](Self::queue_frame) shows it.
    pub fn render_frame<E>(
        &mut self,
        renderer: &mut PixmanRenderer,
        elements: &[E],
        clear: Color32F,
    ) -> Result<DumbFrame, DumbError>
    where
        E: RenderElement<PixmanRenderer>,
    {
        self.ready = None;
        if self.pending.is_some() {
            return Ok(DumbFrame {
                rendered: false,
                states: RenderElementStates::default(),
            });
        }
        let index = match (0..self.slots.len()).find(|i| Some(*i) != self.front) {
            Some(i) => i,
            None => {
                let slot = self.new_slot()?;
                self.slots.push(slot);
                self.slots.len() - 1
            }
        };
        let modeset = self.surface.commit_pending();
        let slot = &mut self.slots[index];
        let age = if modeset { 0 } else { slot.age as usize };
        let mut target = renderer.bind(&mut slot.image).map_err(DumbError::Bind)?;
        let result = self
            .damage
            .render_output(renderer, &mut target, age, elements, clear)
            .map_err(DumbError::Render)?;
        let rendered = modeset || result.damage.is_some_and(|d| !d.is_empty());
        if rendered {
            self.ready = Some(index);
        }
        Ok(DumbFrame {
            rendered,
            states: result.states,
        })
    }

    /// Flip to the buffer the last [`render_frame`](Self::render_frame) drew (a modeset when
    /// the CRTC needs one); a vblank event follows.
    pub fn queue_frame(&mut self, user_data: U) -> Result<(), DumbError> {
        let Some(index) = self.ready.take() else {
            return Ok(());
        };
        let (w, h) = self.size;
        let plane = PlaneState {
            handle: self.surface.plane(),
            config: Some(PlaneConfig {
                src: Rectangle::from_size((w as f64, h as f64).into()),
                dst: Rectangle::from_size((w as i32, h as i32).into()),
                transform: Transform::Normal,
                alpha: 1.0,
                damage_clips: None,
                fb: *self.slots[index].fb.as_ref(),
                fence: None,
            }),
        };
        if self.surface.commit_pending() {
            self.surface.commit([plane], true)?;
        } else {
            self.surface.page_flip([plane], true)?;
        }
        self.pending = Some((index, user_data));
        Ok(())
    }

    /// The flip queued by [`render_frame`](Self::render_frame) completed: its buffer is on
    /// screen. Returns the frame's user data.
    pub fn frame_submitted(&mut self) -> Option<U> {
        let (index, data) = self.pending.take()?;
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if i == index {
                slot.age = 1;
            } else if slot.age > 0 {
                slot.age = slot.age.saturating_add(1);
            }
        }
        self.front = Some(index);
        self.retired.clear();
        Some(data)
    }
}
