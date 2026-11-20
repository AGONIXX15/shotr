use std::{
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Error, Result, anyhow};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, FrameCallbackData},
    delegate_registry,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{EventLoop, LoopHandle},
        calloop_wayland_source::WaylandSource,
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers},
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
    },
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use tiny_skia::{Pixmap, Point};
use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop,
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface::WlSurface},
};
use wayland_protocols::wp::{
    text_input::zv3::client::{
        zwp_text_input_manager_v3::ZwpTextInputManagerV3,
        zwp_text_input_v3::{self, ChangeCause, ContentHint, ContentPurpose, ZwpTextInputV3},
    },
    viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter},
};

use crate::{
    config::{
        APP_ICON, BYTES_PER_PIXEL, IME_SURROUNDING_MAX, MSG_COPIED, MSG_FAILED, MSG_NO_OUTPUT,
        MSG_PATH_COPIED, MSG_SAVED, NAMESPACE, OUTPUT_ROUNDTRIPS, PNG_MIME, STDIO_PATH, TEXT_MIME,
        ZENITY_COLOR_ARGS, ZENITY_SAVE_ARGS,
    },
    editor::{Editor, Request},
    font::Fonts,
    output,
    settings::{Act, ExitTrigger, Settings, Thumbnail, exits_after},
    style::Color,
    text::clip_window,
    tools::Mods,
    util::{self, Stopwatch},
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Outcome {
    Done,
    Cancelled,
}

// Runs the actions in order; true when the app should exit. A cancelled dialog ends the chain.
fn run_chain(
    acts: &[Act],
    early_exit: &[ExitTrigger],
    mut perform: impl FnMut(Act) -> Result<Outcome>,
) -> Result<bool> {
    for &act in acts {
        match perform(act)? {
            Outcome::Cancelled => return Ok(false),
            Outcome::Done if exits_after(early_exit, act) => return Ok(true),
            Outcome::Done => {}
        }
    }
    Ok(false)
}

// events received since the last Done
#[derive(Default)]
struct ImePending {
    commit: String,
    preedit: String,
    delete: (u32, u32),
}

// what the compositor has been told
#[derive(Default)]
struct ImeState {
    focused: bool,
    enabled: bool,
    // the serial of a Done is only valid when it equals the commits sent
    commits: u32,
    sent: Option<(String, usize, [i32; 4])>,
    cause: Option<ChangeCause>,
}

struct Overlay {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    conn: Connection,
    qh: QueueHandle<Overlay>,
    loop_handle: LoopHandle<'static, Overlay>,
    surface: WlSurface,
    layer: Option<LayerSurface>,
    viewport: WpViewport,
    text_input_manager: Option<ZwpTextInputManagerV3>,
    text_input: Option<ZwpTextInputV3>,
    ime: ImeState,
    ime_pending: ImePending,

    settings: Settings,
    editor: Editor,
    screen: (u32, u32),
    scale: (f64, f64),
    last_saved: Option<PathBuf>,
    toolbars_before_leave: Option<bool>,
    stopwatch: Stopwatch,
    frame_pending: bool,
    dirty: bool,
    exit: Option<Result<()>>,
}

pub fn run(
    monitor: Option<&str>,
    base: Pixmap,
    fonts: Fonts,
    settings: Settings,
    stopwatch: Stopwatch,
) -> Result<()> {
    let conn = Connection::connect_to_env().context("cannot connect to Wayland")?;
    let (globals, mut queue) = registry_queue_init(&conn).context("could not list globals")?;
    let qh = queue.handle();
    let mut event_loop: EventLoop<'static, Overlay> =
        EventLoop::try_new().context("could not create the event loop")?;

    let compositor = CompositorState::bind(&globals, &qh).context("wl_compositor missing")?;
    let layer_shell = LayerShell::bind(&globals, &qh).context("wlr-layer-shell missing")?;
    let shm = Shm::bind(&globals, &qh).context("wl_shm missing")?;
    let viewporter: WpViewporter = globals
        .bind(&qh, 1..=1, ())
        .context("wp_viewporter missing")?;

    let surface = compositor.create_surface(&qh);
    let viewport = viewporter.get_viewport(&surface, &qh, ());
    let image = (base.width(), base.height());
    let editor = Editor::new(base, fonts, &settings, settings.input.is_some())?;
    let mut overlay = Overlay {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        pool: SlotPool::new(image.0 as usize * image.1 as usize * BYTES_PER_PIXEL, &shm)
            .context("could not create the shm pool")?,
        shm,
        conn: conn.clone(),
        qh: qh.clone(),
        loop_handle: event_loop.handle(),
        surface: surface.clone(),
        layer: None,
        viewport,
        text_input_manager: globals.bind(&qh, 1..=1, ()).ok(),
        text_input: None,
        ime: ImeState::default(),
        ime_pending: ImePending::default(),
        settings,
        editor,
        screen: image,
        scale: (1.0, 1.0),
        last_saved: None,
        toolbars_before_leave: None,
        stopwatch,
        frame_pending: false,
        dirty: false,
        exit: None,
    };

    // the second roundtrip delivers the output names
    for _ in 0..OUTPUT_ROUNDTRIPS {
        queue
            .roundtrip(&mut overlay)
            .context("initial roundtrip failed")?;
    }
    let state = &overlay.output_state;
    let output = monitor.and_then(|name| {
        state
            .outputs()
            .find(|o| state.info(o).and_then(|i| i.name).as_deref() == Some(name))
    });
    if monitor.is_some() && output.is_none() {
        let desktop = overlay.settings.notifications;
        util::notify(desktop, NAMESPACE, MSG_NO_OUTPUT, None);
    }
    // a loaded image is not the size of the screen, so ask the output for it
    if overlay.settings.input.is_some() {
        let mode = output
            .iter()
            .chain(state.outputs().next().iter())
            .find_map(|o| {
                let info = state.info(o)?;
                info.modes.iter().find(|m| m.current).map(|m| m.dimensions)
            });
        if let Some((w, h)) = mode {
            overlay.screen = (u32::try_from(w)?, u32::try_from(h)?);
            overlay.editor.set_screen(overlay.screen)?;
            overlay.pool = SlotPool::new(
                overlay.screen.0 as usize * overlay.screen.1 as usize * BYTES_PER_PIXEL,
                &overlay.shm,
            )
            .context("could not create the shm pool")?;
        }
    }

    let layer = layer_shell.create_layer_surface(
        &qh,
        surface,
        Layer::Overlay,
        Some(NAMESPACE),
        output.as_ref(),
    );
    layer.set_anchor(Anchor::all());
    layer.set_exclusive_zone(-1);
    layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
    layer.commit();
    overlay.layer = Some(layer);

    WaylandSource::new(conn.clone(), queue)
        .insert(event_loop.handle())
        .map_err(|e| anyhow!("could not register the Wayland queue: {e}"))?;
    while overlay.exit.is_none() {
        event_loop
            .dispatch(None, &mut overlay)
            .context("event loop failed")?;
    }
    overlay.exit.take().unwrap_or(Ok(()))
}

impl Overlay {
    fn handle(&mut self, request: Request) {
        match request {
            Request::PickColor => {
                if let Err(e) = self.pick_color() {
                    self.report(&e);
                }
            }
            Request::Run(acts) => self.run_actions(&acts),
        }
    }

    fn run_actions(&mut self, acts: &[Act]) {
        let early_exit = self.settings.early_exit.clone();
        match run_chain(acts, &early_exit, |act| self.perform(act)) {
            Ok(true) => self.exit = Some(Ok(())),
            Ok(false) => {}
            Err(e) => self.report(&e),
        }
    }

    fn report(&self, error: &Error) {
        let message = format!("{error:#}");
        util::notify(self.settings.notifications, MSG_FAILED, &message, None);
    }

    // `shot` is the saved file or the thumbnail, used when the icon is the screenshot
    fn say(&self, message: &str, shot: Option<&Path>) {
        let icon = match self.settings.thumbnail {
            None => None,
            Some(Thumbnail::AppIcon) => Some(APP_ICON.to_owned()),
            Some(Thumbnail::Screenshot) => shot.map(|p| p.to_string_lossy().into_owned()),
        };
        let desktop = self.settings.notifications;
        util::notify(desktop, NAMESPACE, message, icon.as_deref());
    }

    fn copy_image(&mut self, announce: bool) -> Result<()> {
        let png = self.editor.export()?;
        output::copy(&png, PNG_MIME, self.settings.copy_command.as_deref())?;
        if announce {
            let wants_shot = self.settings.thumbnail == Some(Thumbnail::Screenshot);
            let shot = if wants_shot {
                Some(output::write_thumbnail(&png)?)
            } else {
                None
            };
            self.say(MSG_COPIED, shot.as_deref());
        }
        Ok(())
    }

    fn auto_copy(&mut self) {
        if !self.settings.auto_copy || !self.editor.annotations_changed() {
            return;
        }
        if let Err(e) = self.copy_image(false) {
            self.report(&e);
        }
        // exporting commits an open draft, which is not a new change
        self.editor.annotations_changed();
    }

    fn perform(&mut self, act: Act) -> Result<Outcome> {
        let copy_command = self.settings.copy_command.clone();
        match act {
            Act::Exit => {}
            Act::SaveToClipboard => {
                self.copy_image(true)?;
                if self.settings.save_after_copy {
                    self.save_file()?;
                }
            }
            Act::SaveToFile => {
                self.save_file()?;
            }
            Act::SaveToFileAs => return self.save_file_as(),
            Act::CopyFilepathToClipboard => {
                let path = match self.last_saved.clone() {
                    Some(path) => path,
                    None => self.save_file()?.context("nothing was saved to a file")?,
                };
                let text = path.to_string_lossy();
                output::copy(text.as_bytes(), TEXT_MIME, copy_command.as_deref())?;
                self.say(MSG_PATH_COPIED, Some(&path));
            }
        }
        Ok(Outcome::Done)
    }

    // None when the output is stdout
    fn save_file(&mut self) -> Result<Option<PathBuf>> {
        let png = self.editor.export()?;
        if self.settings.output_filename == STDIO_PATH {
            std::io::stdout()
                .write_all(&png)
                .context("could not write to stdout")?;
            return Ok(None);
        }
        let path = output::save(&png, &self.settings.output_filename)?;
        self.say(&format!("{MSG_SAVED} {}", path.display()), Some(&path));
        self.last_saved = Some(path.clone());
        Ok(Some(path))
    }

    fn save_file_as(&mut self) -> Result<Outcome> {
        let png = self.editor.export()?;
        let suggested = output::suggested_path(&self.settings.output_filename)?;
        let filename = format!("--filename={}", suggested.display());
        let args = [&ZENITY_SAVE_ARGS[..], &[filename.as_str()]].concat();
        let Some(chosen) = self.with_dialog(|| util::zenity(&args))? else {
            return Ok(Outcome::Cancelled);
        };
        let path = PathBuf::from(chosen);
        output::save_to(&png, &path)?;
        self.say(&format!("{MSG_SAVED} {}", path.display()), Some(&path));
        self.last_saved = Some(path);
        Ok(Outcome::Done)
    }

    fn pick_color(&mut self) -> Result<()> {
        let picked = self.with_dialog(|| util::zenity(&ZENITY_COLOR_ARGS))?;
        if let Some(color) = picked.as_deref().and_then(Color::parse) {
            self.editor.set_color(color);
        }
        Ok(())
    }

    // zenity is a normal window, so push the layer below it meanwhile
    fn with_dialog<T>(&self, dialog: impl FnOnce() -> Result<T>) -> Result<T> {
        self.set_stacking(Layer::Bottom, KeyboardInteractivity::None)?;
        let result = dialog();
        if let Err(e) = self.set_stacking(Layer::Overlay, KeyboardInteractivity::Exclusive) {
            self.report(&e);
        }
        result
    }

    fn set_stacking(&self, layer: Layer, keyboard: KeyboardInteractivity) -> Result<()> {
        if let Some(l) = &self.layer {
            l.set_layer(layer);
            l.set_keyboard_interactivity(keyboard);
            l.commit();
        }
        self.conn.flush().context("could not send the layer change")
    }

    // at most one frame in flight
    fn request_draw(&mut self) {
        self.sync_ime();
        if self.exit.is_some() {
            return;
        }
        if self.frame_pending {
            self.dirty = true;
        } else if let Err(e) = self.draw() {
            self.exit = Some(Err(e));
        }
    }

    fn draw(&mut self) -> Result<()> {
        let (w, h) = self.screen;
        let stride = w as usize * BYTES_PER_PIXEL;
        let (buffer, canvas) = self
            .pool
            .create_buffer(w as i32, h as i32, stride as i32, wl_shm::Format::Argb8888)
            .context("could not create the buffer")?;

        let frame = self.editor.render();
        // tiny-skia is RGBA, Argb8888 is BGRA in memory
        let pixels = frame.data().as_chunks::<BYTES_PER_PIXEL>().0;
        for (dst, src) in canvas
            .as_chunks_mut::<BYTES_PER_PIXEL>()
            .0
            .iter_mut()
            .zip(pixels)
        {
            *dst = [src[2], src[1], src[0], src[3]];
        }

        self.surface.damage_buffer(0, 0, w as i32, h as i32);
        self.surface
            .frame(&self.qh, FrameCallbackData(self.surface.clone()));
        buffer
            .attach_to(&self.surface)
            .context("could not attach the buffer")?;
        self.surface.commit();
        self.stopwatch.first_frame();
        self.frame_pending = true;
        self.dirty = false;
        Ok(())
    }

    // Keeps the input method on while a text draft is open, with its popup at the caret.
    fn sync_ime(&mut self) {
        let Some(input) = &self.text_input else {
            return;
        };
        let state = &mut self.ime;
        let draft = self.editor.ime_cursor().zip(self.editor.ime_surrounding());
        let Some((caret, (text, cursor))) = draft.filter(|_| state.focused) else {
            if state.enabled {
                input.disable();
                input.commit();
                state.commits += 1;
                state.enabled = false;
                state.sent = None;
            }
            return;
        };
        let (sx, sy) = self.scale;
        let [x, y, w, h] = caret;
        let rect = [
            (f64::from(x) / sx) as i32,
            (f64::from(y) / sy) as i32,
            (f64::from(w) / sx) as i32,
            (f64::from(h) / sy) as i32,
        ];
        let (window, cursor) = clip_window(&text, cursor, IME_SURROUNDING_MAX);
        let snapshot = (window.to_owned(), cursor, rect);
        if state.enabled && state.sent.as_ref() == Some(&snapshot) {
            return;
        }
        if !state.enabled {
            input.enable();
            state.enabled = true;
        }
        let offset = i32::try_from(cursor).unwrap_or(i32::MAX);
        input.set_surrounding_text(snapshot.0.clone(), offset, offset);
        input.set_text_change_cause(state.cause.take().unwrap_or(ChangeCause::Other));
        input.set_content_type(ContentHint::None, ContentPurpose::Normal);
        input.set_cursor_rectangle(rect[0], rect[1], rect[2], rect[3]);
        input.commit();
        state.commits += 1;
        state.sent = Some(snapshot);
    }

    fn apply_ime(&mut self) {
        let pending = std::mem::take(&mut self.ime_pending);
        let delete = (pending.delete.0 as usize, pending.delete.1 as usize);
        self.editor
            .ime_update(delete, &pending.commit, &pending.preedit);
        self.ime.cause = Some(ChangeCause::InputMethod);
        self.request_draw();
    }

    fn to_physical(&self, (x, y): (f64, f64)) -> Point {
        Point::from_xy((x * self.scale.0) as f32, (y * self.scale.1) as f32)
    }

    fn on_key(&mut self, event: &KeyEvent) {
        let request = self
            .editor
            .key(event.keysym, event.raw_code, event.utf8.as_deref());
        self.after_input(request);
        self.request_draw();
    }

    // A requested action may already have copied, so only plain edits auto-copy.
    fn after_input(&mut self, request: Option<Request>) {
        if let Some(request) = request {
            self.handle(request);
            self.editor.annotations_changed();
        } else {
            self.auto_copy();
        }
    }

    fn on_repeat(&mut self, event: &KeyEvent) {
        if self
            .editor
            .repeat(event.keysym, event.raw_code, event.utf8.as_deref())
        {
            self.request_draw();
        }
    }
}

impl CompositorHandler for Overlay {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: i32,
    ) {
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: u32) {
        self.frame_pending = false;
        if self.dirty {
            self.request_draw();
        }
    }
}

impl OutputHandler for Overlay {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for Overlay {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.exit = Some(Ok(()));
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let (w, h) = configure.new_size;
        if w == 0 || h == 0 {
            return;
        }
        self.viewport.set_destination(w as i32, h as i32);
        self.scale = (
            f64::from(self.screen.0) / f64::from(w),
            f64::from(self.screen.1) / f64::from(h),
        );
        self.request_draw();
    }
}

impl SeatHandler for Overlay {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        _: Capability,
    ) {
    }

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        // proxies outlive their handles
        let result = match capability {
            Capability::Keyboard => {
                if self.text_input.is_none() {
                    self.text_input = self
                        .text_input_manager
                        .as_ref()
                        .map(|manager| manager.get_text_input(&seat, qh, ()));
                }
                let repeat =
                    Box::new(|state: &mut Self, _: &_, event: KeyEvent| state.on_repeat(&event));
                let handle = self.loop_handle.clone();
                self.seat_state
                    .get_keyboard_with_repeat(qh, &seat, None, handle, repeat)
                    .map(drop)
                    .map_err(Error::from)
            }
            Capability::Pointer => self
                .seat_state
                .get_pointer(qh, &seat)
                .map(drop)
                .map_err(Error::from),
            _ => Ok(()),
        };
        if let Err(e) = result {
            self.exit = Some(Err(e));
        }
    }
}

impl KeyboardHandler for Overlay {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
        if !self.settings.focus_toggles_toolbars {
            return;
        }
        if let Some(shown) = self.toolbars_before_leave.take() {
            self.editor.set_toolbars(shown);
            self.request_draw();
        }
    }
    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &WlSurface,
        _: u32,
    ) {
        if self.settings.focus_toggles_toolbars {
            let shown = self.editor.toolbars_shown();
            self.toolbars_before_leave.get_or_insert(shown);
            self.editor.set_toolbars(false);
            self.request_draw();
        }
    }
    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: KeyEvent,
    ) {
    }

    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        self.on_key(&event);
    }

    fn repeat_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        self.on_repeat(&event);
    }

    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        modifiers: Modifiers,
        _: RawModifiers,
        _: u32,
    ) {
        let Modifiers {
            shift, alt, ctrl, ..
        } = modifiers;
        self.editor.set_mods(Mods { shift, alt, ctrl });
    }
}

impl PointerHandler for Overlay {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        let surface = self.surface.clone();
        let mut changed = false;
        for event in events.iter().filter(|e| e.surface == surface) {
            let at = self.to_physical(event.position);
            match event.kind {
                PointerEventKind::Press { button, .. } => {
                    changed = true;
                    let request = self.editor.press(at, button);
                    self.after_input(request);
                }
                PointerEventKind::Release { button, .. } => {
                    changed = true;
                    self.editor.release(button);
                    self.auto_copy();
                }
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    changed |= self.editor.motion(at);
                }
                PointerEventKind::Axis {
                    horizontal,
                    vertical,
                    ..
                } => {
                    changed = true;
                    let amount = (horizontal.absolute as f32, vertical.absolute as f32);
                    self.editor.scroll(at, amount);
                }
                PointerEventKind::Leave { .. } => {}
            }
        }
        if changed {
            self.request_draw();
        }
    }
}

impl ShmHandler for Overlay {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_noop!(Overlay: WpViewporter);
delegate_noop!(Overlay: ZwpTextInputManagerV3);
delegate_noop!(Overlay: WpViewport);
impl Dispatch<ZwpTextInputV3, ()> for Overlay {
    fn event(
        state: &mut Self,
        _: &ZwpTextInputV3,
        event: zwp_text_input_v3::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_text_input_v3::Event::Enter { .. } => {
                let commits = state.ime.commits;
                state.ime = ImeState {
                    focused: true,
                    commits,
                    ..ImeState::default()
                };
                state.ime_pending = ImePending::default();
                state.request_draw();
            }
            zwp_text_input_v3::Event::Leave { .. } => {
                state.editor.ime_update((0, 0), "", "");
                let commits = state.ime.commits;
                state.ime = ImeState {
                    commits,
                    ..ImeState::default()
                };
                state.ime_pending = ImePending::default();
                state.request_draw();
            }
            zwp_text_input_v3::Event::PreeditString { text, .. } => {
                state.ime_pending.preedit = text.unwrap_or_default();
            }
            zwp_text_input_v3::Event::CommitString { text } => {
                state.ime_pending.commit = text.unwrap_or_default();
            }
            zwp_text_input_v3::Event::DeleteSurroundingText {
                before_length,
                after_length,
            } => {
                state.ime_pending.delete = (before_length, after_length);
            }
            zwp_text_input_v3::Event::Done { serial } if serial == state.ime.commits => {
                state.apply_ime();
            }
            zwp_text_input_v3::Event::Done { .. } => state.ime_pending = ImePending::default(),
            _ => {}
        }
    }
}

delegate_registry!(Overlay);

impl ProvidesRegistryState for Overlay {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

smithay_client_toolkit::delegate_dispatch2!(Overlay);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_dialog_stops_the_chain() {
        let mut seen = Vec::new();
        let acts = [Act::SaveToClipboard, Act::SaveToFileAs, Act::Exit];
        let exit = run_chain(&acts, &[], |act| {
            seen.push(act);
            Ok(if act == Act::SaveToFileAs {
                Outcome::Cancelled
            } else {
                Outcome::Done
            })
        });
        assert!(!exit.unwrap());
        assert_eq!(seen, [Act::SaveToClipboard, Act::SaveToFileAs]);
    }

    #[test]
    fn chain_exits_on_exit_or_early_exit() {
        let done = |_| Ok(Outcome::Done);
        assert!(run_chain(&[Act::SaveToFile, Act::Exit], &[], done).unwrap());
        assert!(run_chain(&[Act::SaveToFile], &[ExitTrigger::Save], done).unwrap());
        assert!(!run_chain(&[Act::SaveToFile], &[ExitTrigger::Copy], done).unwrap());
    }
}
