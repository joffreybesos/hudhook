use imgui::internal::RawWrapper;
use imgui::{DrawCmd, DrawVert};
use log::trace;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct3D::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;
use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11DeviceContext};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R16_UINT, DXGI_FORMAT_R32_UINT};
use windows::Win32::Graphics::Dxgi::IDXGISwapChain;

use crate::buffers::Buffers;
use crate::device_and_swapchain::*;
use crate::shader_program::ShaderProgram;
use crate::state_backup::StateBackup;
use crate::texture::Texture;

pub struct RenderEngine {
    ctx: imgui::Context,
    dasc: DeviceAndSwapChain,
    shader_program: ShaderProgram,
    buffers: Buffers,
    texture: Texture,
}

impl RenderEngine {
    pub fn new(hwnd: HWND) -> Self {
        let mut ctx = imgui::Context::create();
        let dasc = DeviceAndSwapChain::new(hwnd);
        let shader_program = ShaderProgram::new(&dasc).expect("ShaderProgram");
        let buffers = Buffers::new(&dasc);
        let texture = Texture::new(&dasc, &mut ctx.fonts()).expect("Texture");
        RenderEngine { ctx, dasc, shader_program, buffers, texture }
    }

    pub fn new_with_ptrs(
        dev: ID3D11Device,
        dev_ctx: ID3D11DeviceContext,
        swap_chain: IDXGISwapChain,
    ) -> Self {
        let mut ctx = imgui::Context::create();
        let dasc = DeviceAndSwapChain::new_with_ptrs(dev, dev_ctx, swap_chain);
        let shader_program = ShaderProgram::new(&dasc).expect("ShaderProgram");
        let buffers = Buffers::new(&dasc);
        let texture = Texture::new(&dasc, &mut ctx.fonts()).expect("Texture");
        RenderEngine { ctx, dasc, shader_program, buffers, texture }
    }

    pub fn ctx(&mut self) -> &mut imgui::Context {
        &mut self.ctx
    }

    pub fn dev(&self) -> ID3D11Device {
        self.dasc.dev()
    }

    pub fn dev_ctx(&self) -> ID3D11DeviceContext {
        self.dasc.dev_ctx()
    }

    pub fn swap_chain(&self) -> IDXGISwapChain {
        self.dasc.swap_chain()
    }

    pub fn render<F: FnOnce(&mut imgui::Ui)>(&mut self, f: F) -> Result<(), String> {
        trace!("Rendering started");
        let state_backup = StateBackup::backup(self.dasc.dev_ctx());

        if let Some(mut rect) = self.dasc.get_window_rect() {
            self.ctx.io_mut().display_size =
                [(rect.right - rect.left) as f32, (rect.bottom - rect.top) as f32];
            rect.right -= rect.left;
            rect.bottom -= rect.top;
            rect.top = 0;
            rect.left = 0;
            self.dasc.set_viewport(rect);
            self.dasc.set_render_target();
        }
        trace!("Set shader program state");
        unsafe { self.shader_program.set_state(&self.dasc) };

        let mut ui = self.ctx.frame();
        f(&mut ui);
        let draw_data = ui.render();

        let [x, y] = draw_data.display_pos;
        let [width, height] = draw_data.display_size;

        if width <= 0. && height <= 0. {
            return Err(format!("Insufficient display size {} x {}", width, height));
        }

        unsafe {
            let dev_ctx = self.dasc.dev_ctx();

            trace!("Setting up buffers");
            self.buffers.set_constant_buffer(&self.dasc, [x, y, x + width, y + height]);
            self.buffers.set_buffers(&self.dasc, draw_data.draw_lists());

            dev_ctx.IASetVertexBuffers(
                0,
                1,
                &Some(self.buffers.vtx_buffer()),
                &(std::mem::size_of::<DrawVert>() as u32),
                &0,
            );
            dev_ctx.IASetIndexBuffer(
                self.buffers.idx_buffer(),
                if std::mem::size_of::<imgui::DrawIdx>() == 2 {
                    DXGI_FORMAT_R16_UINT
                } else {
                    DXGI_FORMAT_R32_UINT
                },
                0,
            );
            dev_ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            dev_ctx.VSSetConstantBuffers(0, &[Some(self.buffers.mtx_buffer())]);
            dev_ctx.PSSetShaderResources(0, &[Some(self.texture.tex_view())]);

            let mut vtx_offset = 0usize;
            let mut idx_offset = 0usize;

            trace!("Rendering draw lists");
            for cl in draw_data.draw_lists() {
                for cmd in cl.commands() {
                    match cmd {
                        DrawCmd::Elements { count, cmd_params } => {
                            trace!("Rendering {count} elements");
                            let [cx, cy, cw, ch] = cmd_params.clip_rect;
                            dev_ctx.RSSetScissorRects(&[RECT {
                                left: (cx - x) as i32,
                                top: (cy - y) as i32,
                                right: (cw - x) as i32,
                                bottom: (ch - y) as i32,
                            }]);

                            // let srv = cmd_params.texture_id.id();
                            // We only load the font texture. This may not be correct.
                            self.dasc.set_shader_resources(self.texture.tex_view());

                            trace!("Drawing indexed {count}, {idx_offset}, {vtx_offset}");
                            dev_ctx.DrawIndexed(count as u32, idx_offset as _, vtx_offset as _);

                            idx_offset += count;
                        },
                        DrawCmd::ResetRenderState => {
                            trace!("Resetting render state");
                            self.dasc.setup_state(draw_data);
                            self.shader_program.set_state(&self.dasc);
                        },
                        DrawCmd::RawCallback { callback, raw_cmd } => {
                            trace!("Executing raw callback");
                            callback(cl.raw(), raw_cmd)
                        },
                    }
                }
                vtx_offset += cl.vtx_buffer().len();
            }

            // self.dasc.swap_chain().Present(1, 0);
        }

        trace!("Restoring state backup");
        state_backup.restore(self.dasc.dev_ctx());

        trace!("Rendering done");

        Ok(())
    }

    pub fn present(&self) {
        if let Err(e) = unsafe { self.dasc.swap_chain().Present(1, 0) } {
            log::error!("Present: {e}");
        }
    }
}