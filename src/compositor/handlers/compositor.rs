//! Compositor and SHM buffer handlers

use crate::compositor::{grabs::resize_grab, state::ClientState, Compositor};
use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    delegate_compositor, delegate_shm,
    reexports::wayland_server::{
        protocol::{wl_buffer, wl_surface::WlSurface},
        Client, Resource,
    },
    wayland::{
        buffer::BufferHandler,
        compositor::{
            get_parent, is_sync_subsurface, CompositorClientState, CompositorHandler,
            CompositorState,
        },
        shm::{ShmHandler, ShmState},
    },
};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;

use super::xdg_shell;

impl CompositorHandler for Compositor {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            if let Some(window) = self
                .space
                .elements()
                .find(|w| w.toplevel().unwrap().wl_surface() == &root)
            {
                window.on_commit();
            }
        };

        xdg_shell::handle_commit(&mut self.popups, &self.space, surface, &mut self.taskbar_dirty);
        resize_grab::handle_commit(&mut self.space, surface);

        let surface_id = surface.id().protocol_id();

        // Find the window and check if it was marked as a dialog at creation time.
        let window_info = self
            .space
            .elements()
            .find(|w| w.toplevel().unwrap().wl_surface() == surface)
            .map(|w| {
                let geo = w.geometry();
                let bbox = w.bbox();
                let is_dialog = self.dialog_surfaces.contains(&surface_id);
                (w.clone(), geo, bbox, is_dialog)
            });

        if let Some((window, geo, bbox, is_dialog)) = window_info {
            let output_size = self.space.outputs().next()
                .and_then(|o| self.space.output_geometry(o))
                .map(|g| g.size);

            if let Some(out_size) = output_size {
                // Log first few commits per surface for debugging geometry
                if !self.titlebar_adjusted.contains(&surface_id) || is_dialog {
                    log::info!(
                        "commit: sid={} dialog={} geo={:?} bbox={:?} loc={:?} output={}x{}",
                        surface_id, is_dialog, geo, bbox,
                        self.space.element_location(&window),
                        out_size.w, out_size.h
                    );
                }
                if is_dialog {
                    // Center dialog using bbox (includes CSD decorations/shadows).
                    let bw = bbox.size.w;
                    let bh = bbox.size.h;
                    if bw > 0 && bh > 0 {
                        if bh > out_size.h - 20 || bw > out_size.w - 20 {
                            // Dialog too large — compute CSD overhead and shrink content
                            let csd_w = (bbox.size.w - geo.size.w).max(0);
                            let csd_h = (bbox.size.h - geo.size.h).max(0);
                            let target_w = out_size.w - csd_w - 40;
                            let target_h = out_size.h - csd_h - 40;
                            let new_w = geo.size.w.min(target_w).max(200);
                            let new_h = geo.size.h.min(target_h).max(200);
                            if new_w != geo.size.w || new_h != geo.size.h {
                                log::info!(
                                    "Dialog shrink: geo={}x{} bbox={}x{} csd={}x{} -> {}x{}",
                                    geo.size.w, geo.size.h, bw, bh, csd_w, csd_h, new_w, new_h
                                );
                                let toplevel = window.toplevel().unwrap();
                                toplevel.with_pending_state(|state| {
                                    state.size = Some((new_w, new_h).into());
                                });
                                toplevel.send_pending_configure();
                            }
                        }
                        // Center based on bbox so CSD shadows stay on screen
                        let x = (out_size.w - bw).max(0) / 2 - bbox.loc.x;
                        let y = (out_size.h - bh).max(0) / 2 - bbox.loc.y;
                        self.space.map_element(window, (x, y), true);
                    }
                } else if !self.titlebar_adjusted.contains(&surface_id) {
                    // Once app_id is known, set Fullscreen for non-browser apps.
                    // Browsers (chromium, firefox, etc.) must NOT get Fullscreen
                    // because they hide their address bar in fullscreen mode.
                    // new_toplevel only sets size; Fullscreen is deferred to here.
                    if !self.browser_unfullscreened.contains(&surface_id) {
                        let app_id = smithay::wayland::compositor::with_states(surface, |states| {
                            states.data_map
                                .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                                .map(|d| d.lock().unwrap().app_id.clone().unwrap_or_default())
                                .unwrap_or_default()
                        });
                        // Decide once app_id is known, OR once the surface has
                        // real geometry (some apps like weston-terminal never set app_id).
                        let has_geometry = geo.size.w > 0 && geo.size.h > 0;
                        if !app_id.is_empty() || has_geometry {
                            self.browser_unfullscreened.insert(surface_id);
                            let is_browser = app_id.contains("chromium") || app_id.contains("firefox")
                                || app_id.contains("google-chrome") || app_id.contains("brave");
                            if !is_browser {
                                let toplevel = window.toplevel().unwrap();
                                toplevel.with_pending_state(|state| {
                                    state.states.set(xdg_toplevel::State::Fullscreen);
                                    state.size = Some((out_size.w, out_size.h).into());
                                });
                                toplevel.send_pending_configure();
                                log::info!("Non-browser app (app_id={}), set Fullscreen for sid={}", app_id, surface_id);
                            } else {
                                log::info!("Browser detected (app_id={}), skipping Fullscreen for sid={}", app_id, surface_id);
                            }
                        }
                    }
                    // CSD compensation for non-dialog toplevels only
                    let titlebar_h = if geo.loc.y > 0 {
                        geo.loc.y
                    } else if geo.size.h > 0 && geo.size.h < out_size.h {
                        out_size.h - geo.size.h
                    } else {
                        0
                    };

                    if titlebar_h > 0 && self.csd_retry_count < 3 {
                        self.csd_retry_count += 1;
                        self.titlebar_adjusted.insert(surface_id);
                        let toplevel = window.toplevel().unwrap();
                        toplevel.with_pending_state(|state| {
                            state.size = Some((out_size.w, out_size.h + titlebar_h).into());
                        });
                        toplevel.send_pending_configure();
                        log::info!(
                            "CSD compensate: surface {} geo={:?} bbox={:?} output={}x{} adding {}px",
                            surface_id, geo, bbox, out_size.w, out_size.h, titlebar_h
                        );
                    }
                }
            }
        }

        self.needs_redraw = true;
    }
}

impl BufferHandler for Compositor {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for Compositor {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

delegate_compositor!(Compositor);
delegate_shm!(Compositor);
