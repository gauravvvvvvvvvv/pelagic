//! Small Windows host. All UI painting and audio synthesis are Rust/CPU code;
//! the operating system supplies the window, file picker and sound output.
use crate::{
    sound,
    view::{INTRO_SECONDS, Model, Painter},
};
use std::{
    cell::RefCell,
    ffi::c_void,
    mem::{size_of, zeroed},
    path::Path,
    ptr::{null, null_mut},
    time::Instant,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Dwm::*, Gdi::*},
    Media::Audio::*,
    System::{Com::*, LibraryLoader::*},
    UI::{Controls::Dialogs::*, HiDpi::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};
#[path = "windows_studio.rs"]
mod studio;
use studio::{poll_job, studio_action};

struct App {
    painter: Painter,
    model: Model,
    started: Instant,
    intro: bool,
    cue: Vec<u8>,
    last_tick: Instant,
    icons: Vec<HICON>,
    animating: bool,
    last_animation: Instant,
    input_cursor: usize,
    input_selected: bool,
    pending_surrogate: Option<u16>,
    drag: Option<(i32, i32, crate::project::Scene)>,
    split_drag: Option<(crate::layout::Splitter, crate::layout::LayoutSettings)>,
    gizmo_drag: Option<crate::viewport_edit::Drag>,
    terrain_job: Option<std::sync::mpsc::Receiver<Result<pelagic::terrain::HeightGrid, String>>>,
    diagnostics: Option<std::path::PathBuf>,
    message_counts: std::collections::BTreeMap<u32, u64>,
    last_diagnostics: Instant,
    job: Option<crate::jobs::Job>,
    close_pending: bool,
    accessibility: Option<crate::accessibility::Accessibility>,
    access_events: Option<accesskit_windows::QueuedEvents>,
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn run(gpu_error: Option<&str>) {
    crate::session::CPU_FRAMES.store(true, std::sync::atomic::Ordering::Relaxed);
    // SAFETY: Win32 handles remain on the UI thread. The boxed App is stable until
    // after WM_QUIT, and all pointers passed to synchronous APIs outlive the call.
    unsafe {
        let com = CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32);
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let instance = GetModuleHandleW(null());
        let class = wide("PelagicNativeLauncher");
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: class.as_ptr(),
            ..zeroed()
        };
        if RegisterClassW(&wc) == 0 {
            return;
        }
        let mut model = Model::default();
        let arguments: Vec<String> = std::env::args().collect();
        if let Some(path) = arguments
            .iter()
            .find_map(|a| a.strip_prefix("--test-storage="))
        {
            model.storage_root = Some(path.into());
        }
        crate::editor::preferences(&mut model);
        model.viewport_maximized = arguments.iter().any(|a| a == "--maximize-viewport");
        if arguments.iter().any(|a| a == "--dark") {
            model.dark_mode = true;
        }
        if arguments.iter().any(|a| a == "--light") {
            model.dark_mode = false;
        }
        if let Some(path) = arguments
            .windows(2)
            .find(|a| a[0] == "--open")
            .map(|a| &a[1])
        {
            match crate::project::Document::open(Path::new(path)) {
                Ok(doc) => crate::editor::install(&mut model, doc),
                Err(error) => model.notice = error,
            }
        }
        if let Some(error) = gpu_error {
            model.notice =
                format!("CPU mode: GPU editor unavailable ({error}). Your project is preserved.");
            model.record_notice();
            let _ = crate::project::atomic_json(
                &crate::editor::storage(&model).join("startup.json"),
                &serde_json::json!({"backend":"cpu", "reason":error}),
            );
        }
        let mut animations: BOOL = 1;
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&mut animations as *mut BOOL).cast(),
            0,
        );
        model.reduced_motion = animations == 0;
        let state = Box::new(RefCell::new(App {
            painter: Painter::new(),
            model,
            started: Instant::now(),
            intro: gpu_error.is_none(),
            cue: sound::startup_wave(),
            last_tick: Instant::now(),
            icons: Vec::new(),
            animating: false,
            last_animation: Instant::now(),
            input_cursor: 0,
            input_selected: false,
            pending_surrogate: None,
            drag: None,
            split_drag: None,
            gizmo_drag: None,
            terrain_job: None,
            diagnostics: std::env::args().find_map(|a| {
                a.strip_prefix("--diagnostics=")
                    .map(std::path::PathBuf::from)
            }),
            message_counts: Default::default(),
            last_diagnostics: Instant::now(),
            job: None,
            close_pending: false,
            accessibility: None,
            access_events: None,
        }));
        let initial_dpi = GetDpiForSystem().max(96) as f32 / 96.0;
        let mut initial_size = [(1120.0 * initial_dpi) as i32, (780.0 * initial_dpi) as i32];
        let mut work_area: RECT = zeroed();
        if SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut work_area as *mut RECT).cast(), 0) != 0 {
            initial_size[0] = initial_size[0].min((work_area.right - work_area.left).max(1));
            initial_size[1] = initial_size[1].min((work_area.bottom - work_area.top).max(1));
        }
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            wide("Pelagic").as_ptr(),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            initial_size[0],
            initial_size[1],
            null_mut(),
            null_mut(),
            instance,
            (&*state as *const RefCell<App>).cast_mut().cast(),
        );
        if hwnd.is_null() {
            return;
        }
        if let Some(path) = arguments
            .iter()
            .find_map(|a| a.strip_prefix("--automation-handle="))
        {
            let _ = std::fs::write(path, format!("{}", hwnd as usize));
        }
        {
            let mut app = state.borrow_mut();
            app.painter.dpi_scale = GetDpiForWindow(hwnd) as f32 / 96.0;
            app.painter.paint(1120, 740, &Model::default(), Some(0.0));
            app.accessibility = Some(crate::accessibility::Accessibility::new(
                hwnd as usize,
                &app.painter,
                &app.model,
            ));
            // Apply the frame before showing the window to avoid a white flash.
            app.icons = vec![
                create_logo_icon(&app.painter, 16, false),
                create_logo_icon(&app.painter, 64, false),
                create_logo_icon(&app.painter, 16, true),
                create_logo_icon(&app.painter, 64, true),
            ];
            apply_window_theme(hwnd, app.model.dark_mode);
            set_theme_icons(hwnd, &app);
            app.started = Instant::now();
            ShowWindow(hwnd, SW_SHOW);
            if !app.model.muted {
                PlaySoundW(
                    app.cue.as_ptr().cast(),
                    null_mut(),
                    SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
                );
            }
            SetTimer(hwnd, 1, 16, None);
        }
        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        PlaySoundW(null(), null_mut(), 0);
        UnregisterClassW(class.as_ptr(), instance);
        for icon in &state.borrow().icons {
            if !icon.is_null() {
                DestroyIcon(*icon);
            }
        }
        if com >= 0 {
            CoUninitialize();
        }
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: The pointer is set once at WM_NCCREATE and remains valid for the
    // entire message loop. Handlers run serially; no callback keeps a reference.
    unsafe {
        // DestroyWindow re-enters this callback while WM_CLOSE owns App.
        // Quit must therefore run before the RefCell reentrancy guard.
        if msg == WM_DESTROY {
            PostQuitMessage(0);
            return 0;
        }
        if msg == WM_NCCREATE {
            let create = &*(l as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<App>;
        if ptr.is_null() {
            return DefWindowProcW(hwnd, msg, w, l);
        }
        // Native dialogs/capture/theme APIs can synchronously re-enter this
        // callback. Never create overlapping mutable Rust references to App.
        let Ok(mut state) = (&*ptr).try_borrow_mut() else {
            return DefWindowProcW(hwnd, msg, w, l);
        };
        let app = &mut *state;
        if let Some(path) = &app.diagnostics {
            *app.message_counts.entry(msg).or_default() += 1;
            if msg == WM_TIMER {
                *app.message_counts.entry(10000 + w as u32).or_default() += 1;
            }
            if app.last_diagnostics.elapsed().as_secs_f64() > 1.0 {
                let _ = std::fs::write(
                    path,
                    format!(
                        "intro={} elapsed={:?} counts={:?} backend={}",
                        app.intro,
                        app.started.elapsed(),
                        app.message_counts,
                        app.model
                            .session
                            .as_ref()
                            .map_or("none", |s| s.backend_name())
                    ),
                );
                app.last_diagnostics = Instant::now();
            }
        }
        let result = match msg {
            crate::accessibility::MESSAGE => {
                let requests: Vec<_> = app
                    .accessibility
                    .as_ref()
                    .map(|a| a.requests.try_iter().collect())
                    .unwrap_or_default();
                for request in requests {
                    let Some(id) = crate::accessibility::target(&request) else {
                        continue;
                    };
                    if app.intro || app.gizmo_drag.is_some() || !app.painter.interactive_id(id) {
                        continue;
                    }
                    if app.model.editing.as_ref().is_some_and(|e| e.0 != id)
                        && let Err(e) = crate::editor::commit(&mut app.model)
                    {
                        app.model.notice = e;
                        continue;
                    }
                    SetFocus(hwnd);
                    app.model.focus = Some(id);
                    match request.action {
                        accesskit::Action::SetValue
                            if crate::layout::Splitter::from_id(id).is_some() =>
                        {
                            if let Some(value) = crate::accessibility::numeric_value(request) {
                                set_panel_value(
                                    hwnd,
                                    app,
                                    crate::layout::Splitter::from_id(id).unwrap(),
                                    value as f32,
                                );
                            }
                        }
                        accesskit::Action::Increment | accesskit::Action::Decrement
                            if crate::layout::Splitter::from_id(id).is_some() =>
                        {
                            let splitter = crate::layout::Splitter::from_id(id).unwrap();
                            let dimensions = workspace_dimensions(hwnd, app);
                            let layout = crate::view::EditorLayout::new(
                                dimensions[0],
                                dimensions[1],
                                app.model.layout,
                                app.model.content_collapsed,
                                false,
                            );
                            let delta = if request.action == accesskit::Action::Increment {
                                20.0
                            } else {
                                -20.0
                            };
                            set_panel_value(hwnd, app, splitter, layout.range(splitter).0 + delta);
                        }
                        accesskit::Action::Click => action(hwnd, app, id),
                        accesskit::Action::Focus => {
                            if crate::editor::is_field(id) {
                                begin_edit(app, id);
                            }
                        }
                        accesskit::Action::SetValue if crate::editor::is_field(id) => {
                            if let Some(value) = crate::accessibility::value(request) {
                                if id != 254 {
                                    pause(hwnd, app);
                                }
                                begin_edit(app, id);
                                app.model.editing = Some((id, value));
                                if let Err(e) = crate::editor::commit(&mut app.model) {
                                    app.model.notice = e;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                if IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 {
                    refresh_semantics(hwnd, app);
                }
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_THEMECHANGED | WM_DWMCOLORIZATIONCOLORCHANGED => {
                apply_window_theme(hwnd, app.model.dark_mode);
                DefWindowProcW(hwnd, msg, w, l)
            }
            WM_ERASEBKGND => 1,
            WM_GETMINMAXINFO => {
                let limits = &mut *(l as *mut MINMAXINFO);
                let dpi = GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
                limits.ptMinTrackSize = POINT {
                    x: (900.0 * dpi) as i32,
                    y: (650.0 * dpi) as i32,
                };
                0
            }
            WM_SIZE => {
                if app.gizmo_drag.is_some() {
                    cancel_transform(app);
                }
                if w == SIZE_MINIMIZED as usize {
                    KillTimer(hwnd, 2);
                    KillTimer(hwnd, 3);
                    app.animating = false;
                    return 0;
                }
                app.last_tick = Instant::now();
                if app.model.session.as_ref().is_some_and(|s| s.running) {
                    SetTimer(hwnd, 2, 33, None);
                }
                update_viewport(hwnd, app);
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_DPICHANGED => {
                app.painter.dpi_scale = (w as u16) as f32 / 96.0;
                let rect = &*(l as *const RECT);
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                0
            }
            WM_SETTINGCHANGE => {
                let mut animations: BOOL = 1;
                SystemParametersInfoW(
                    SPI_GETCLIENTAREAANIMATION,
                    0,
                    (&mut animations as *mut BOOL).cast(),
                    0,
                );
                app.model.reduced_motion = animations == 0;
                animate_controls(hwnd, app);
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_TIMER => {
                if w == 6 {
                    let mut pending = vec![];
                    for receiver in app.model.pending_saves.drain(..) {
                        match receiver.try_recv() {
                            Ok(Ok(path)) => {
                                app.model.notice =
                                    format!("Recording backed up: {}", path.display())
                            }
                            Ok(Err(error)) => {
                                app.model.notice = format!(
                                    "Run backup failed: {}. Kept in memory; use Experiments > Run actions > Retry failed backups.",
                                    error.message
                                );
                                app.model.failed_runs.push(error.run);
                                app.model.editing = None;
                                if let Some(session) = &mut app.model.session {
                                    session.running = false;
                                }
                                KillTimer(hwnd, 2);
                                app.close_pending = false;
                            }
                            Err(std::sync::mpsc::TryRecvError::Empty) => pending.push(receiver),
                            Err(_) => {
                                app.model.notice =
                                    "Run backup worker stopped; export the loaded recording."
                                        .into();
                                app.close_pending = false;
                            }
                        }
                    }
                    app.model.pending_saves = pending;
                    if app.model.pending_saves.is_empty() {
                        KillTimer(hwnd, 6);
                        if app.close_pending {
                            PostMessageW(hwnd, WM_CLOSE, 0, 0);
                        }
                    }
                } else if w == 5 {
                    let was_running = app.job.is_some();
                    poll_job(hwnd, app);
                    if was_running
                        && app.job.is_none()
                        && (IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0)
                    {
                        refresh_semantics(hwnd, app);
                    }
                    if !app.model.pending_saves.is_empty() {
                        SetTimer(hwnd, 6, 100, None);
                    }
                } else if w == 4 {
                    let result = app
                        .terrain_job
                        .as_ref()
                        .and_then(|job| match job.try_recv() {
                            Ok(result) => Some(result),
                            Err(std::sync::mpsc::TryRecvError::Empty) => None,
                            Err(_) => Some(Err("Terrain import stopped.".into())),
                        });
                    if let Some(result) = result {
                        cancel_import(hwnd, app);
                        match result {
                            Ok(grid) => {
                                if let Some(doc) = &app.model.document {
                                    let mut scene = doc.scene.clone();
                                    scene.grid = Some(std::sync::Arc::new(grid));
                                    scene.terrain = 3;
                                    if let Err(error) =
                                        crate::editor::edit(&mut app.model, scene, true)
                                    {
                                        app.model.notice = error;
                                    } else {
                                        app.model.notice="Imported terrain is embedded in this project. Save to keep it.".into();
                                    }
                                }
                            }
                            Err(error) => app.model.notice = error,
                        }
                        update_viewport(hwnd, app);
                    }
                } else if w == 3 {
                    let elapsed = app.last_animation.elapsed().as_secs_f32();
                    app.last_animation = Instant::now();
                    if !app.painter.animate_buttons(&app.model, elapsed) {
                        KillTimer(hwnd, 3);
                        app.animating = false;
                    }
                } else if w == 2 {
                    let work = Instant::now();
                    let elapsed = app.last_tick.elapsed().as_secs_f64();
                    app.last_tick = Instant::now();
                    if IsIconic(hwnd) == 0
                        && let Some(session) = app.model.session.as_mut()
                    {
                        session.advance_view(
                            elapsed,
                            app.model.viewport_mode < 2 || app.model.viewport_mode == 6,
                        );
                    }
                    app.model.frame_update_ms = work.elapsed().as_secs_f64() * 1000.0;
                } else if w == 1 && app.started.elapsed().as_secs_f64() >= INTRO_SECONDS {
                    finish_intro(hwnd, app);
                }
                if IsIconic(hwnd) == 0 {
                    InvalidateRect(hwnd, null(), 0);
                }
                0
            }
            WM_PAINT => {
                let paint_start = Instant::now();
                if let Some(session) = app.model.session.as_mut()
                    && !session.backend_notice.is_empty()
                {
                    app.model.notice = std::mem::take(&mut session.backend_notice);
                }
                let mut ps: PAINTSTRUCT = zeroed();
                let dc = BeginPaint(hwnd, &mut ps);
                let mut rect: RECT = zeroed();
                GetClientRect(hwnd, &mut rect);
                let width = (rect.right - rect.left).max(1) as usize;
                let height = (rect.bottom - rect.top).max(1) as usize;
                app.model.input_cursor = app.input_cursor;
                app.model.input_selected = app.input_selected;
                app.painter.paint(
                    width,
                    height,
                    &app.model,
                    app.intro.then(|| app.started.elapsed().as_secs_f64()),
                );
                let bitmap = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: width as i32,
                        biHeight: -(height as i32),
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB,
                        ..zeroed()
                    },
                    ..zeroed()
                };
                StretchDIBits(
                    dc,
                    0,
                    0,
                    width as i32,
                    height as i32,
                    0,
                    0,
                    width as i32,
                    height as i32,
                    app.painter.pixels.as_ptr().cast::<c_void>(),
                    &bitmap,
                    DIB_RGB_COLORS,
                    SRCCOPY,
                );
                EndPaint(hwnd, &ps);
                app.model.last_frame_ms =
                    app.model.frame_update_ms + paint_start.elapsed().as_secs_f64() * 1000.0;
                app.model.frame_update_ms = 0.0;
                app.access_events = app
                    .accessibility
                    .as_mut()
                    .and_then(|a| a.update(&app.painter, &app.model));
                0
            }
            WM_MOUSEMOVE => {
                let mut tracking = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                TrackMouseEvent(&mut tracking);
                let x = (l as u16 as i16) as f32;
                let y = ((l >> 16) as u16 as i16) as f32;
                if let Some(drag) = app.gizmo_drag.as_mut() {
                    let point = [x / app.painter.ui_scale(), y / app.painter.ui_scale()];
                    match drag.update(point) {
                        Ok(scene) => {
                            if let Some(session) = app.model.session.as_mut()
                                && let Err(error) = session.preview_scene(&scene)
                            {
                                app.model.notice = error;
                            }
                            app.model.transform_preview = Some(scene);
                        }
                        Err(error) => app.model.notice = error,
                    }
                    InvalidateRect(hwnd, null(), 0);
                    return 0;
                }
                if let Some((splitter, _)) = app.split_drag {
                    let size = workspace_dimensions(hwnd, app);
                    let layout = crate::view::EditorLayout::new(
                        size[0],
                        size[1],
                        app.model.layout,
                        app.model.content_collapsed,
                        false,
                    );
                    let scale = app.painter.dpi_scale.max(0.5);
                    layout.drag(&mut app.model.layout, splitter, [x / scale, y / scale]);
                    SetCursor(LoadCursorW(
                        null_mut(),
                        if splitter == crate::layout::Splitter::Sidebar {
                            IDC_SIZEWE
                        } else {
                            IDC_SIZENS
                        },
                    ));
                    InvalidateRect(hwnd, null(), 0);
                    return 0;
                }
                if let Some((last_x, last_y, scene)) = app.drag.take() {
                    let next = crate::editor::orbit_scene(
                        &scene,
                        x as f64 - last_x as f64,
                        y as f64 - last_y as f64,
                    );
                    if let Some(session) = app.model.session.as_mut() {
                        session.set_camera(&next);
                    }
                    app.drag = Some((x as i32, y as i32, next));
                    InvalidateRect(hwnd, null(), 0);
                    return 0;
                }
                let hover = app.painter.hit(x, y);
                if hover != app.model.hover {
                    app.model.hover = hover;
                    animate_controls(hwnd, app);
                    InvalidateRect(hwnd, null(), 0);
                }
                if let Some(splitter) = app.painter.splitter_at(x, y) {
                    SetCursor(LoadCursorW(
                        null_mut(),
                        if splitter == crate::layout::Splitter::Sidebar {
                            IDC_SIZEWE
                        } else {
                            IDC_SIZENS
                        },
                    ));
                } else if hover.is_some() {
                    SetCursor(LoadCursorW(null_mut(), IDC_HAND));
                }
                0
            }
            0x02a3 => {
                // WM_MOUSELEAVE
                app.model.hover = None;
                animate_controls(hwnd, app);
                0
            }
            WM_RBUTTONDOWN => {
                if app.gizmo_drag.is_some() || app.split_drag.is_some() {
                    return 0;
                }
                let x = l as u16 as i16 as i32;
                let y = (l >> 16) as u16 as i16 as i32;
                if app.model.workspace
                    && app.painter.viewport_contains(x as f32, y as f32)
                    && let Some(doc) = &app.model.document
                {
                    app.drag = Some((x, y, doc.scene.clone()));
                    SetCapture(hwnd);
                }
                0
            }
            WM_RBUTTONUP => {
                if let Some((_, _, scene)) = app.drag.take() {
                    ReleaseCapture();
                    if let Err(error) = crate::editor::edit(&mut app.model, scene, false) {
                        app.model.notice = error;
                    }
                    InvalidateRect(hwnd, null(), 0);
                }
                0
            }
            WM_MOUSEWHEEL => {
                if app.gizmo_drag.is_some() {
                    return 0;
                }
                let mut cursor = POINT {
                    x: l as u16 as i16 as i32,
                    y: (l >> 16) as u16 as i16 as i32,
                };
                ScreenToClient(hwnd, &mut cursor);
                if app.model.workspace
                    && app
                        .painter
                        .outliner_contains(cursor.x as f32, cursor.y as f32)
                {
                    let delta = (w >> 16) as u16 as i16;
                    app.model.outliner_scroll = if delta > 0 {
                        app.model.outliner_scroll.saturating_sub(1)
                    } else {
                        (app.model.outliner_scroll + 1).min(20)
                    };
                    InvalidateRect(hwnd, null(), 0);
                    return 0;
                }
                if app.model.workspace
                    && app.model.editing.is_none()
                    && app
                        .painter
                        .viewport_contains(cursor.x as f32, cursor.y as f32)
                {
                    let delta = (w >> 16) as u16 as i16 as f64 / 120.0;
                    if let Err(error) = crate::editor::camera(&mut app.model, 0, delta) {
                        app.model.notice = error;
                    }
                    InvalidateRect(hwnd, null(), 0);
                }
                0
            }
            WM_LBUTTONDOWN => {
                if !app.intro {
                    if app.drag.is_some() {
                        return 0;
                    }
                    if let Some(splitter) = app
                        .painter
                        .splitter_at((l as u16 as i16) as f32, ((l >> 16) as u16 as i16) as f32)
                    {
                        if let Err(error) = crate::editor::commit(&mut app.model) {
                            app.model.notice = error;
                            return 0;
                        }
                        app.model.focus = Some(splitter.id());
                        app.model.pressed = None;
                        app.split_drag = Some((splitter, app.model.layout));
                        SetCapture(hwnd);
                        return 0;
                    }
                    let had_edit = app.model.editing.is_some();
                    let hit = app
                        .painter
                        .hit((l as u16 as i16) as f32, ((l >> 16) as u16 as i16) as f32);
                    if app
                        .model
                        .editing
                        .as_ref()
                        .is_some_and(|edit| Some(edit.0) != hit)
                        && hit != Some(24)
                        && let Err(error) = crate::editor::commit(&mut app.model)
                    {
                        app.model.notice = error;
                        InvalidateRect(hwnd, null(), 0);
                        return 0;
                    }
                    if had_edit {
                        update_viewport(hwnd, app);
                        refresh_semantics(hwnd, app);
                    }
                    let point = [
                        (l as u16 as i16) as f32 / app.painter.ui_scale(),
                        ((l >> 16) as u16 as i16) as f32 / app.painter.ui_scale(),
                    ];
                    if let Some(gizmo) = app.painter.gizmo.as_ref()
                        && let Some(handle) = gizmo.hit(point)
                        && crate::viewport_edit::can_edit(&app.model)
                        && let Some(doc) = &app.model.document
                    {
                        app.gizmo_drag = crate::viewport_edit::Drag::begin(
                            doc.scene.clone(),
                            gizmo.actor,
                            handle,
                            gizmo.camera,
                            point,
                            app.model.transform_snap,
                        );
                        if app.gizmo_drag.is_some() {
                            app.model.focus = Some(handle.id());
                            app.model.pressed = None;
                            SetCapture(hwnd);
                            return 0;
                        }
                    }
                    if hit.is_none()
                        && let Some(camera) = app.painter.viewport_camera
                        && camera.contains(point)
                    {
                        let session = app.model.session.as_ref().unwrap();
                        let actor = crate::viewport_edit::pick(
                            camera,
                            point,
                            &session.view_snapshot(),
                            session.view_ocean(),
                        );
                        let height = app.painter.editor_layout.map_or(200., |l| l.outliner[3]);
                        crate::viewport_edit::select(&mut app.model, actor, height);
                        app.model.focus = None;
                        app.model.pressed = None;
                        update_viewport(hwnd, app);
                        if IsWindowVisible(hwnd) == 0 {
                            refresh_semantics(hwnd, app);
                        }
                        InvalidateRect(hwnd, null(), 0);
                        return 0;
                    }
                    app.model.pressed = hit;
                    app.model.focus = hit;
                    animate_controls(hwnd, app);
                    if hit.is_some() {
                        SetCapture(hwnd);
                    }
                    InvalidateRect(hwnd, null(), 0);
                }
                0
            }
            WM_CAPTURECHANGED | WM_KILLFOCUS => {
                if app.gizmo_drag.is_some() {
                    cancel_transform(app);
                }
                if app.split_drag.take().is_some() {
                    save_workspace(app);
                    update_viewport(hwnd, app);
                }
                app.model.pressed = None;
                if msg == WM_KILLFOCUS {
                    app.model.focus = None;
                }
                if let Some((_, _, scene)) = app.drag.take()
                    && let Err(error) = crate::editor::edit(&mut app.model, scene, false)
                {
                    app.model.notice = error;
                }
                animate_controls(hwnd, app);
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_LBUTTONUP => {
                if !app.intro {
                    if let Some(drag) = app.gizmo_drag.take() {
                        ReleaseCapture();
                        if let Some(next) = app.model.transform_preview.take()
                            && let Err(error) =
                                crate::editor::commit_transform(&mut app.model, &drag.base, next)
                        {
                            app.model.notice = error;
                        }
                        if let Some(session) = app.model.session.as_mut() {
                            session.clear_preview();
                        }
                        update_viewport(hwnd, app);
                        if IsWindowVisible(hwnd) == 0 {
                            refresh_semantics(hwnd, app);
                        }
                        InvalidateRect(hwnd, null(), 0);
                        return 0;
                    }
                    if app.split_drag.take().is_some() {
                        ReleaseCapture();
                        save_workspace(app);
                        update_viewport(hwnd, app);
                        if IsWindowVisible(hwnd) == 0 {
                            refresh_semantics(hwnd, app);
                        }
                        InvalidateRect(hwnd, null(), 0);
                        return 0;
                    }
                    let hit = app
                        .painter
                        .hit((l as u16 as i16) as f32, ((l >> 16) as u16 as i16) as f32);
                    let clicked = hit.filter(|id| app.model.pressed == Some(*id));
                    app.model.pressed = None;
                    ReleaseCapture();
                    if let Some(id) = clicked {
                        app.model.focus = Some(id);
                        action(hwnd, app, id);
                    }
                    InvalidateRect(hwnd, null(), 0);
                }
                0
            }
            WM_KEYDOWN => {
                handle_key(hwnd, app, w);
                animate_controls(hwnd, app);
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_CHAR => {
                if app.model.editing.is_some() && GetKeyState(0x11) >= 0 {
                    let unit = w as u16;
                    let character = if (0xd800..=0xdbff).contains(&unit) {
                        app.pending_surrogate = Some(unit);
                        None
                    } else if (0xdc00..=0xdfff).contains(&unit) {
                        app.pending_surrogate.take().and_then(|hi| {
                            char::decode_utf16([hi, unit]).next().and_then(Result::ok)
                        })
                    } else {
                        app.pending_surrogate = None;
                        char::from_u32(w as u32)
                    };
                    if let Some(ch) = character.filter(|c| !c.is_control()) {
                        insert_text(app, &ch.to_string());
                    }
                    InvalidateRect(hwnd, null(), 0);
                }
                0
            }
            WM_CLOSE => {
                if !app.model.failed_runs.is_empty() {
                    app.model.notice="Unsaved recording recovery is in memory. Retry failed backups before closing.".into();
                    app.model.inspector_page = 4;
                    InvalidateRect(hwnd, null(), 0);
                    return 0;
                }
                if app.close_pending && app.model.pending_saves.is_empty() {
                    DestroyWindow(hwnd);
                    return 0;
                }
                if app.model.job_running {
                    app.model.notice =
                        "Wait for the background task or cancel it in Run actions before closing."
                            .into();
                    InvalidateRect(hwnd, null(), 0);
                    return 0;
                }
                if let Err(error) = crate::editor::commit(&mut app.model) {
                    app.model.notice = error;
                    InvalidateRect(hwnd, null(), 0);
                    return 0;
                }
                // Keep a recovery snapshot even when the user chooses to exit
                // without saving; explicit Save removes it after durable write.
                if app.model.document.as_ref().is_some_and(|d| d.dirty()) {
                    pause(hwnd, app);
                    let choice=MessageBoxW(hwnd,wide("Save your scene before closing?\nNo keeps an unsaved recovery copy for the next launch.").as_ptr(),wide("Unsaved changes").as_ptr(),MB_YESNOCANCEL|MB_ICONQUESTION);
                    if choice == IDCANCEL {
                        return 0;
                    }
                    if choice == IDYES && !save_document(hwnd, app, false) {
                        return 0;
                    }
                    if choice == IDNO
                        && let Err(error) = crate::editor::recovery(&mut app.model)
                    {
                        MessageBoxW(
                            hwnd,
                            wide(&error).as_ptr(),
                            wide("Recovery failed — scene remains open").as_ptr(),
                            MB_OK | MB_ICONERROR,
                        );
                        return 0;
                    }
                }
                let _ = crate::editor::save_preferences(&app.model);
                crate::editor::archive_run(&mut app.model);
                if !app.model.pending_saves.is_empty() {
                    app.close_pending = true;
                    SetTimer(hwnd, 6, 100, None);
                    app.model.notice = "Saving recording before closing...".into();
                    return 0;
                }
                PlaySoundW(null(), null_mut(), 0);
                KillTimer(hwnd, 1);
                KillTimer(hwnd, 2);
                KillTimer(hwnd, 3);
                KillTimer(hwnd, 4);
                DestroyWindow(hwnd);
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        };
        app.model.record_notice();
        let events = app.access_events.take();
        drop(state);
        if let Some(events) = events {
            events.raise();
        }
        result
    }
}

unsafe fn animate_controls(hwnd: HWND, app: &mut App) {
    unsafe {
        if app.model.reduced_motion {
            app.painter.animate_buttons(&app.model, 0.0);
            KillTimer(hwnd, 3);
            app.animating = false;
            return;
        }
        if !app.animating {
            app.animating = true;
            app.last_animation = Instant::now();
            SetTimer(hwnd, 3, 16, None);
        }
    }
}
unsafe fn cancel_import(hwnd: HWND, app: &mut App) {
    unsafe {
        KillTimer(hwnd, 4);
    }
    app.terrain_job = None;
    app.model.importing = false;
}
unsafe fn update_viewport(hwnd: HWND, app: &mut App) {
    let selected = if app.model.inspector_page == 1 {
        app.model
            .selected_entity
            .and_then(|i| {
                app.model
                    .document
                    .as_ref()?
                    .scene
                    .entities
                    .get(i)
                    .map(|e| e.id)
            })
            .or(Some(0))
    } else {
        None
    };
    if let Some(session) = app.model.session.as_mut() {
        session.view(app.model.viewport_mode == 6, selected);
    }
    if !app.model.workspace || app.model.new_project || (2..=5).contains(&app.model.viewport_mode) {
        return;
    }
    unsafe {
        let mut rect: RECT = zeroed();
        GetClientRect(hwnd, &mut rect);
        let width = (rect.right - rect.left).max(1) as f64;
        let height = (rect.bottom - rect.top).max(1) as f64;
        let scale = app.painter.dpi_scale.max(0.5) as f64;
        let layout = crate::view::EditorLayout::new(
            (width / scale) as f32,
            (height / scale) as f32,
            app.model.layout,
            app.model.content_collapsed,
            app.model.viewport_maximized,
        );
        let viewport_width = layout.viewport[2] as f64 * scale;
        let viewport_height = layout.viewport[3] as f64 * scale;
        let quality = app
            .model
            .document
            .as_ref()
            .map_or(1, |d| d.scene.quality as usize);
        let target = viewport_width
            .min([512.0, 800.0, 1120.0][quality])
            .max(320.0) as usize;
        let h = (target as f64 * viewport_height / viewport_width).clamp(180.0, 900.0) as usize;
        if let Some(session) = app.model.session.as_mut() {
            session.resize(target, h);
        }
    }
}
unsafe fn workspace_dimensions(hwnd: HWND, app: &App) -> [f32; 2] {
    unsafe {
        let mut rect: RECT = zeroed();
        GetClientRect(hwnd, &mut rect);
        let scale = app.painter.dpi_scale.max(0.5);
        [
            (rect.right - rect.left).max(1) as f32 / scale,
            (rect.bottom - rect.top).max(1) as f32 / scale,
        ]
    }
}
fn save_workspace(app: &mut App) {
    if let Err(error) = crate::editor::save_preferences(&app.model) {
        app.model.notice = format!("Workspace preferences could not be saved: {error}");
    }
}
fn cancel_transform(app: &mut App) {
    app.gizmo_drag = None;
    app.model.transform_preview = None;
    if let Some(session) = app.model.session.as_mut() {
        session.clear_preview();
    }
    app.model.pressed = None;
}
fn frame_actor(app: &mut App) -> Result<(), String> {
    use pelagic::math::Vec3;
    let actor =
        crate::viewport_edit::Actor::selected(&app.model).ok_or("Select an actor first.")?;
    if !matches!(app.model.viewport_mode, 0 | 1) {
        return Err("Frame selected is available in surface and underwater scene views.".into());
    }
    let mut scene = app
        .model
        .document
        .as_ref()
        .ok_or("No scene open.")?
        .scene
        .clone();
    let state = app.model.session.as_ref().unwrap().view_snapshot();
    let (position, radius) = if let Some(body) = state
        .vehicles
        .iter()
        .find(|p| Some(p.id) == actor.runtime_id())
    {
        (body.position, body.half.length())
    } else {
        (
            actor.position(&scene).ok_or("Actor was removed.")?,
            15. * scene.structure_scale,
        )
    };
    let eye = Vec3::new(scene.camera[0], scene.camera[1], scene.camera[2]);
    let target = Vec3::new(scene.target[0], scene.target[1], scene.target[2]);
    let mut direction = (eye - target).normalized();
    if direction.length() < 0.9 {
        direction = Vec3::new(-1., -1., 0.6).normalized();
    }
    let next = position + direction * (radius * 4.).max(4.);
    scene.camera = [next.x, next.y, next.z];
    scene.target = [position.x, position.y, position.z];
    crate::editor::edit(&mut app.model, scene, false)
}
unsafe fn set_panel_value(
    hwnd: HWND,
    app: &mut App,
    splitter: crate::layout::Splitter,
    value: f32,
) {
    unsafe {
        let dimensions = workspace_dimensions(hwnd, app);
        if let Err(error) = crate::editor::resize_panel(&mut app.model, dimensions, splitter, value)
        {
            app.model.notice = format!("Workspace preferences could not be saved: {error}");
        }
        update_viewport(hwnd, app);
        InvalidateRect(hwnd, null(), 0);
    }
}
unsafe fn pick_menu(hwnd: HWND, labels: &[&str], selected: usize) -> Option<usize> {
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return None;
        }
        for (i, label) in labels.iter().enumerate() {
            AppendMenuW(
                menu,
                MF_STRING | if i == selected { MF_CHECKED } else { 0 },
                100 + i,
                wide(label).as_ptr(),
            );
        }
        let mut cursor: POINT = zeroed();
        GetCursorPos(&mut cursor);
        let choice = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY,
            cursor.x,
            cursor.y,
            0,
            hwnd,
            null(),
        );
        DestroyMenu(menu);
        if choice >= 100 && (choice as usize) < 100 + labels.len() {
            Some(choice as usize - 100)
        } else {
            None
        }
    }
}
unsafe fn pick_terrain(hwnd: HWND) -> Option<String> {
    unsafe {
        let mut path = vec![0_u16; 32768];
        let filter = wide("Elevation grid (*.csv;*.txt)\0*.csv;*.txt\0");
        let title = wide("Import SI elevation rows (maximum 257 x 257)");
        let mut dialog = OPENFILENAMEW {
            lStructSize: size_of::<OPENFILENAMEW>() as u32,
            hwndOwner: hwnd,
            lpstrFile: path.as_mut_ptr(),
            nMaxFile: path.len() as u32,
            lpstrFilter: filter.as_ptr(),
            lpstrTitle: title.as_ptr(),
            Flags: OFN_EXPLORER | OFN_NOCHANGEDIR | OFN_FILEMUSTEXIST,
            ..zeroed()
        };
        if GetOpenFileNameW(&mut dialog) == 0 {
            None
        } else {
            let n = path.iter().position(|c| *c == 0).unwrap_or(path.len());
            Some(String::from_utf16_lossy(&path[..n]))
        }
    }
}

/// Native caption buttons, resizing, dragging and snap behaviour are preserved.
unsafe fn apply_window_theme(hwnd: HWND, _dark: bool) {
    unsafe {
        let enabled: i32 = 1;
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            (&enabled as *const i32).cast(),
            size_of::<i32>() as u32,
        );
        // COLORREF is 0x00BBGGRR, unlike the CPU renderer's 0x00RRGGBB.
        for (attribute, color) in [
            (DWMWA_CAPTION_COLOR, 0x000000_u32),
            (DWMWA_TEXT_COLOR, 0xffffff_u32),
            (DWMWA_BORDER_COLOR, 0x262626_u32),
        ] {
            // Explicit caption colours are available on Windows 11. Windows 10
            // keeps the immersive dark caption when these attributes are absent.
            DwmSetWindowAttribute(
                hwnd,
                attribute as u32,
                (&color as *const u32).cast(),
                size_of::<u32>() as u32,
            );
        }
        SetWindowTextW(hwnd, wide("Pelagic — Ocean Engine").as_ptr());
        SetWindowPos(
            hwnd,
            null_mut(),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
}

unsafe fn set_theme_icons(hwnd: HWND, app: &App) {
    unsafe {
        // Windows taskbars can be dark even while the workspace is light.
        // Always use the white brand mark for the OS-managed icon slots.
        let i = 2;
        SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, app.icons[i] as isize);
        SendMessageW(
            hwnd,
            WM_SETICON,
            ICON_BIG as usize,
            app.icons[i + 1] as isize,
        );
    }
}

unsafe fn create_logo_icon(painter: &Painter, side: usize, dark: bool) -> HICON {
    unsafe {
        let mut pixels = painter.icon_pixels(side);
        if !dark {
            for pixel in &mut pixels {
                *pixel &= 0xff000000;
            }
        }
        let bitmap_info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: side as i32,
                biHeight: -(side as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..zeroed()
            },
            ..zeroed()
        };
        let mut bits = null_mut();
        let color = CreateDIBSection(
            null_mut(),
            &bitmap_info,
            DIB_RGB_COLORS,
            &mut bits,
            null_mut(),
            0,
        );
        if color.is_null() {
            return null_mut();
        }
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u32>(), pixels.len());
        let mask_data = vec![0_u8; side.div_ceil(16) * 2 * side];
        let mask = CreateBitmap(side as i32, side as i32, 1, 1, mask_data.as_ptr().cast());
        if mask.is_null() {
            DeleteObject(color);
            return null_mut();
        }
        let info = ICONINFO {
            fIcon: 1,
            hbmColor: color,
            hbmMask: mask,
            ..zeroed()
        };
        let icon = CreateIconIndirect(&info);
        // CreateIconIndirect copies the bitmaps; only the HICON outlives this call.
        DeleteObject(color);
        DeleteObject(mask);
        icon
    }
}

unsafe fn finish_intro(hwnd: HWND, app: &mut App) {
    unsafe {
        app.intro = false;
        PlaySoundW(null(), null_mut(), 0);
        KillTimer(hwnd, 1);
        if std::env::args().any(|a| a == "--gpu") {
            studio::request_gpu_preview(hwnd, app);
        }
        if IsWindowVisible(hwnd) == 0 {
            refresh_semantics(hwnd, app);
        }
        InvalidateRect(hwnd, null(), 0);
    }
}
fn refresh_semantics(hwnd: HWND, app: &mut App) {
    let mut rect: RECT = unsafe { zeroed() };
    // The hidden test window still has a real client size. Use it so UIA bounds
    // and splitter ranges agree with the geometry used by native input.
    unsafe {
        GetClientRect(hwnd, &mut rect);
    }
    app.painter.paint(
        if rect.right > 0 {
            rect.right as usize
        } else {
            app.painter.width.max(900)
        },
        if rect.bottom > 0 {
            rect.bottom as usize
        } else {
            app.painter.height.max(620)
        },
        &app.model,
        None,
    );
    app.access_events = app
        .accessibility
        .as_mut()
        .and_then(|a| a.update(&app.painter, &app.model));
}

unsafe fn action(hwnd: HWND, app: &mut App, id: usize) {
    unsafe {
        if app.gizmo_drag.is_some() {
            return;
        }
        if matches!(id,14|15|32|90..=94|121|249|255|270|300..=316|346|347|377|383|400..=416) {
            app.model.selection_empty = false;
        }
        if (!app.model.failed_runs.is_empty() || app.model.pending_saves.len() >= 4)
            && !matches!(id, 7 | 16 | 19 | 92 | 101)
        {
            app.model.notice="Finish or retry recording backups. Experiments > Run actions > Retry failed backups.".into();
            InvalidateRect(hwnd, null(), 0);
            return;
        }
        if id >= 90 {
            studio_action(hwnd, app, id);
            animate_controls(hwnd, app);
            update_viewport(hwnd, app);
            if !app.model.pending_saves.is_empty() {
                SetTimer(hwnd, 6, 100, None);
            }
            InvalidateRect(hwnd, null(), 0);
            return;
        }
        if id == 13 {
            crate::editor::archive_run(&mut app.model);
            app.model.playback = None;
        }
        if matches!(id, 2 | 3 | 8 | 27 | 60..=63) {
            cancel_import(hwnd, app);
        }
        if id == 24 {
            app.model.editing = None;
            app.model.new_project = false;
            app.model.notice.clear();
        } else {
            if app.model.editing.as_ref().is_some_and(|edit| edit.0 != id)
                && let Err(error) = crate::editor::commit(&mut app.model)
            {
                app.model.notice = error;
                InvalidateRect(hwnd, null(), 0);
                return;
            }
            match id {
                0 => app.model.about = false,
                1 => app.model.about = true,
                2 => {
                    if !confirm_replace(hwnd, app) {
                        return;
                    }
                    pause(hwnd, app);
                    app.model.new_project = true;
                    app.model.new_name = "Untitled ocean".into();
                    app.model.new_folder = std::env::current_exe()
                        .ok()
                        .and_then(|p| p.parent().map(|d| d.join("Projects")))
                        .unwrap_or_else(|| {
                            std::path::PathBuf::from("C:\\Users\\Public\\Documents\\Pelagic")
                        })
                        .to_string_lossy()
                        .into();
                    app.model.new_vehicle = false;
                    app.model.notice.clear();
                    begin_edit(app, 20);
                }
                3 => {
                    if !confirm_replace(hwnd, app) {
                        return;
                    }
                    pause(hwnd, app);
                    if let Some(path) = pick_project(hwnd, false) {
                        open_document(app, &path);
                    }
                }
                4 => {
                    app.intro = true;
                    app.started = Instant::now();
                    app.model.hover = None;
                    if !app.model.muted {
                        PlaySoundW(
                            app.cue.as_ptr().cast(),
                            null_mut(),
                            SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
                        );
                    }
                    SetTimer(hwnd, 1, 16, None);
                }
                5 => {
                    app.model.muted = !app.model.muted;
                    let _ = crate::editor::save_preferences(&app.model);
                }
                6 => {
                    app.model.workspace = app.model.session.is_some();
                    app.model.focus = None;
                }
                7 => {
                    app.model.dark_mode = !app.model.dark_mode;
                    apply_window_theme(hwnd, app.model.dark_mode);
                    set_theme_icons(hwnd, app);
                    let _ = crate::editor::save_preferences(&app.model);
                }
                8 => {
                    if !confirm_replace(hwnd, app) {
                        return;
                    }
                    pause(hwnd, app);
                    let scene = crate::project::Scene::robotics_demo();
                    crate::editor::install(&mut app.model, crate::project::Document::new(scene));
                    if let Err(error) = crate::editor::recovery(&mut app.model) {
                        app.model.notice = error;
                    }
                }
                10 => {
                    pause(hwnd, app);
                    app.model.workspace = false;
                    app.model.focus = None;
                }
                11 => {
                    if app.model.playback.take().is_some() {
                        app.model.notice =
                            "Playback ended; continuing the separate live scene.".into();
                        if let Some(s) = app.model.session.as_mut() {
                            s.display = None;
                        }
                    }
                    if let Some(session) = app.model.session.as_mut() {
                        session.toggle();
                        if session.running {
                            app.last_tick = Instant::now();
                            SetTimer(hwnd, 2, 33, None);
                        } else {
                            KillTimer(hwnd, 2);
                        }
                    }
                }
                12 => {
                    app.model.playback = None;
                    pause(hwnd, app);
                    if let Some(session) = app.model.session.as_mut() {
                        session.step();
                    }
                }
                13 => {
                    pause(hwnd, app);
                    if let (Some(session), Some(doc)) =
                        (&mut app.model.session, &app.model.document)
                    {
                        session.reset_scene(&doc.scene);
                    }
                }
                14 => {
                    app.model.inspector_page = 0;
                    app.model.inspect_ocean = true;
                    app.model.inspect_structure = false;
                }
                15 => {
                    app.model.inspector_page = 1;
                    app.model.tool_tab = 0;
                    app.model.inspect_ocean = false;
                    app.model.inspect_structure = false;
                }
                16 => {
                    save_document(hwnd, app, false);
                }
                19 => {
                    save_document(hwnd, app, true);
                }
                17 | 18 => {
                    pause(hwnd, app);
                    if let Err(error) = crate::editor::history(&mut app.model, id == 18) {
                        app.model.notice = error;
                    }
                }
                id if crate::editor::is_field(id) => {
                    pause(hwnd, app);
                    begin_edit(app, id);
                }
                22 => {
                    if let Some(path) = pick_folder(hwnd) {
                        app.model.new_folder = path;
                    }
                }
                23 => match crate::editor::create(&mut app.model) {
                    Ok(()) => {}
                    Err(error) => app.model.notice = error,
                },
                25 => app.model.new_vehicle = false,
                26 => app.model.new_vehicle = true,
                27 => {
                    if confirm_replace(hwnd, app) {
                        pause(hwnd, app);
                        if let Err(error) = crate::editor::recover(&mut app.model) {
                            app.model.notice = error;
                        }
                    }
                }
                60..=63 => {
                    if let Some(path) = app.model.recent.get(id - 60).cloned()
                        && confirm_replace(hwnd, app)
                    {
                        pause(hwnd, app);
                        open_document(app, &path);
                    }
                }
                46 => {
                    pause(hwnd, app);
                    if let Some(doc) = &app.model.document {
                        let mut scene = doc.scene.clone();
                        scene.vehicle = !scene.vehicle;
                        if scene.vehicle_depth >= scene.bottom_depth {
                            scene.vehicle_depth = scene.bottom_depth * 0.5;
                        }
                        if let Err(error) = crate::editor::edit(&mut app.model, scene, true) {
                            app.model.notice = error;
                        }
                    }
                }
                32 => {
                    if let Some(tab) = pick_menu(
                        hwnd,
                        &[
                            "Water settings",
                            "Sea state / waves",
                            "Current field",
                            "Terrain",
                            "Water profile",
                        ],
                        app.model.environment_tab,
                    ) {
                        app.model.environment_tab = tab;
                    }
                }
                33 => {
                    if let Some(kind) = pick_menu(
                        hwnd,
                        &["Flat seabed", "Coastal shelf", "Canyon", "Imported grid"],
                        app.model
                            .document
                            .as_ref()
                            .map_or(0, |d| d.scene.terrain as usize),
                    ) && let Some(doc) = &app.model.document
                    {
                        let mut scene = doc.scene.clone();
                        scene.terrain = kind as u8;
                        if let Err(error) = crate::editor::edit(&mut app.model, scene, true) {
                            app.model.notice = error;
                        }
                    }
                }
                34 => {
                    app.model.inspector_page = 0;
                    app.model.inspect_structure = true;
                }
                35 => {
                    if let Some(doc) = &app.model.document {
                        let mut scene = doc.scene.clone();
                        scene.structure = !scene.structure;
                        if let Err(error) = crate::editor::edit(&mut app.model, scene, true) {
                            app.model.notice = error;
                        }
                    }
                }
                47 | 48 => {
                    pause(hwnd, app);
                    app.model.viewport_mode = if id == 47 { 0 } else { 1 };
                    if let Some(doc) = &app.model.document {
                        let mut scene = doc.scene.clone();
                        if id == 47 {
                            scene.camera = [-95.0, -125.0, 34.0];
                            scene.target = [0.0, 15.0, 0.0];
                        } else {
                            let depth = scene.vehicle_depth.min(scene.bottom_depth * 0.7);
                            scene.camera = [-7.0, -10.0, -depth + 5.0];
                            scene.target = [0.0, 0.0, -depth];
                        }
                        if let Err(error) = crate::editor::edit(&mut app.model, scene, false) {
                            app.model.notice = error;
                        }
                    }
                }
                49 => app.model.viewport_mode = 2,
                57 => {
                    app.model.wireframe = !app.model.wireframe;
                    if let Some(session) = app.model.session.as_mut() {
                        session.wireframe(app.model.wireframe);
                    }
                }
                58 => {
                    if let Some(quality) = pick_menu(
                        hwnd,
                        &["Low / CPU budget", "Balanced", "High detail"],
                        app.model
                            .document
                            .as_ref()
                            .map_or(1, |d| d.scene.quality as usize),
                    ) && let Some(doc) = &app.model.document
                    {
                        let mut scene = doc.scene.clone();
                        scene.quality = quality as u8;
                        if let Err(error) = crate::editor::edit(&mut app.model, scene, false) {
                            app.model.notice = error;
                        }
                    }
                }
                59 => {
                    if let Some(preset) = pick_menu(
                        hwnd,
                        &[
                            "Calm",
                            "Gentle / wind 5 m/s",
                            "Moderate / wind 8 m/s",
                            "Fresh / wind 12 m/s",
                        ],
                        0,
                    ) && let Some(doc) = &app.model.document
                    {
                        let mut scene = doc.scene.clone();
                        let wind = [0.0, 5.0, 8.0, 12.0][preset];
                        scene.wave_height = 0.21 * wind * wind / 9.80665;
                        scene.wave_period = if wind == 0.0 { 6.0 } else { 0.83 * wind };
                        if let Err(error) = crate::editor::edit(&mut app.model, scene, true) {
                            app.model.notice = error;
                        } else {
                            app.model.notice="Wind-derived initial sea state; wind does not force wave growth during a run.".into();
                        }
                    }
                }
                64 => {
                    if let Some(material) = pick_menu(
                        hwnd,
                        &["Automatic / sand and rock", "Sand", "Rock"],
                        app.model
                            .document
                            .as_ref()
                            .map_or(0, |d| d.scene.terrain_material as usize),
                    ) && let Some(doc) = &app.model.document
                    {
                        let mut scene = doc.scene.clone();
                        scene.terrain_material = material as u8;
                        if let Err(error) = crate::editor::edit(&mut app.model, scene, false) {
                            app.model.notice = error;
                        }
                    }
                }
                67 => {
                    if let Some(path) = pick_terrain(hwnd) {
                        pause(hwnd, app);
                        cancel_import(hwnd, app);
                        let (send, receive) = std::sync::mpsc::channel();
                        app.terrain_job = Some(receive);
                        app.model.importing = true;
                        app.model.notice =
                            "Importing terrain... you can cancel from the inspector.".into();
                        std::thread::spawn(move || {
                            let result = (|| {
                                use std::io::Read;
                                let mut text = String::new();
                                std::fs::File::open(path)
                                    .map_err(|e| e.to_string())?
                                    .take(8_388_609)
                                    .read_to_string(&mut text)
                                    .map_err(|e| e.to_string())?;
                                if text.len() > 8_388_608 {
                                    return Err("Terrain CSV exceeds 8 MB.".into());
                                }
                                let grid = if text.trim_start().to_lowercase().starts_with("ncols")
                                {
                                    pelagic::data::esri_ascii(&text).map_err(str::to_string)?
                                } else {
                                    pelagic::terrain::HeightGrid::parse_csv(
                                        &text,
                                        [-256.0, -256.0],
                                        4.0,
                                    )?
                                };
                                if grid.columns > 257 || grid.rows > 257 {
                                    return Err("Terrain must be at most 257 x 257 samples.".into());
                                }
                                Ok(grid)
                            })();
                            let _ = send.send(result);
                        });
                        SetTimer(hwnd, 4, 50, None);
                    }
                }
                69 => {
                    cancel_import(hwnd, app);
                    app.model.notice = "Terrain import cancelled.".into();
                }
                _ => {}
            }
        }
        animate_controls(hwnd, app);
        update_viewport(hwnd, app);
        if !app.model.pending_saves.is_empty() {
            SetTimer(hwnd, 6, 100, None);
        }
        InvalidateRect(hwnd, null(), 0);
    }
}
fn begin_edit(app: &mut App, id: usize) {
    let value = crate::editor::field_value(&app.model, id);
    app.input_cursor = value.len();
    app.input_selected = true;
    app.model.editing = Some((id, value));
    app.model.focus = Some(id);
}
fn insert_text(app: &mut App, text: &str) {
    if let Some((_, buffer)) = app.model.editing.as_mut() {
        let text: String = text
            .chars()
            .filter(|c| !c.is_control())
            .take(2048)
            .collect();
        if app.input_selected {
            buffer.clear();
            app.input_cursor = 0;
            app.input_selected = false;
        }
        let cursor = app.input_cursor.min(buffer.len());
        if buffer.len() + text.len() <= 8192 && buffer.is_char_boundary(cursor) {
            buffer.insert_str(cursor, &text);
            app.input_cursor = cursor + text.len();
        }
    }
}
unsafe fn paste_field(hwnd: HWND, app: &mut App) {
    unsafe {
        use windows_sys::Win32::System::{DataExchange::*, Memory::*};
        if OpenClipboard(hwnd) == 0 {
            return;
        }
        let handle = GetClipboardData(13); // CF_UNICODETEXT, only on user's Ctrl+V.
        if !handle.is_null() {
            let ptr = GlobalLock(handle).cast::<u16>();
            if !ptr.is_null() {
                let maximum = (GlobalSize(handle) / 2).min(8192);
                let units = std::slice::from_raw_parts(ptr, maximum);
                let n = units.iter().position(|c| *c == 0).unwrap_or(units.len());
                let text = String::from_utf16_lossy(&units[..n]);
                insert_text(app, &text);
                GlobalUnlock(handle);
            }
        }
        CloseClipboard();
    }
}
unsafe fn handle_key(hwnd: HWND, app: &mut App, key: usize) {
    unsafe {
        if app.intro {
            if key == 27 {
                finish_intro(hwnd, app);
            }
            return;
        }
        let ctrl = GetKeyState(0x11) < 0;
        let shift = GetKeyState(0x10) < 0;
        if app.gizmo_drag.is_some() {
            if key == 27 {
                cancel_transform(app);
                ReleaseCapture();
            }
            return;
        }
        if let Some((_, original)) = app.split_drag {
            if key == 27 {
                app.model.layout = original;
                app.split_drag = None;
                ReleaseCapture();
                update_viewport(hwnd, app);
            }
            return;
        }
        if key == 0x7a && app.model.workspace {
            action(hwnd, app, 269);
            return;
        }
        if app.model.editing.is_none()
            && let Some(splitter) = app.model.focus.and_then(crate::layout::Splitter::from_id)
        {
            let delta = match (splitter, key) {
                (crate::layout::Splitter::Sidebar, 37)
                | (crate::layout::Splitter::Outliner, 40)
                | (crate::layout::Splitter::Content, 38) => Some(20.0),
                (crate::layout::Splitter::Sidebar, 39)
                | (crate::layout::Splitter::Outliner, 38)
                | (crate::layout::Splitter::Content, 40) => Some(-20.0),
                _ => None,
            };
            if let Some(delta) = delta {
                let dimensions = workspace_dimensions(hwnd, app);
                let layout = crate::view::EditorLayout::new(
                    dimensions[0],
                    dimensions[1],
                    app.model.layout,
                    app.model.content_collapsed,
                    false,
                );
                set_panel_value(hwnd, app, splitter, layout.range(splitter).0 + delta);
                return;
            }
        }
        if ctrl {
            match key {
                32 if app.model.workspace && app.model.editing.is_none() => action(hwnd, app, 246),
                65 if app.model.editing.is_some() => app.input_selected = true,
                86 if app.model.editing.is_some() => paste_field(hwnd, app),
                83 => action(hwnd, app, if shift { 19 } else { 16 }),
                78 => action(hwnd, app, 2),
                79 => action(hwnd, app, 3),
                90 if app.model.editing.is_some() => {
                    app.model.editing = None;
                }
                90 => action(hwnd, app, if shift { 18 } else { 17 }),
                89 => action(hwnd, app, 18),
                13 if app.model.new_project => action(hwnd, app, 23),
                _ => {}
            }
            return;
        }
        if key == 27 {
            if app.model.editing.take().is_none() && app.model.new_project {
                action(hwnd, app, 24);
            }
            return;
        }
        if key == 9 {
            if let Err(error) = crate::editor::commit(&mut app.model) {
                app.model.notice = error;
                return;
            }
            let ids: Vec<usize> = app.painter.focus_ids();
            if ids.is_empty() {
                return;
            }
            let next = app
                .model
                .focus
                .and_then(|id| ids.iter().position(|v| *v == id))
                .map_or(0, |i| {
                    if shift {
                        (i + ids.len() - 1) % ids.len()
                    } else {
                        (i + 1) % ids.len()
                    }
                });
            let id = ids[next];
            app.model.focus = Some(id);
            if crate::editor::is_field(id) {
                begin_edit(app, id);
            }
            return;
        }
        if let Some((_, buffer)) = app.model.editing.as_mut() {
            match key {
                8 | 46 => {
                    if app.input_selected {
                        buffer.clear();
                        app.input_cursor = 0;
                        app.input_selected = false;
                    } else if key == 8 && app.input_cursor > 0 {
                        let start = buffer[..app.input_cursor]
                            .char_indices()
                            .last()
                            .map(|p| p.0)
                            .unwrap_or(0);
                        buffer.drain(start..app.input_cursor);
                        app.input_cursor = start;
                    } else if key == 46 && app.input_cursor < buffer.len() {
                        let end = app.input_cursor
                            + buffer[app.input_cursor..]
                                .chars()
                                .next()
                                .unwrap()
                                .len_utf8();
                        buffer.drain(app.input_cursor..end);
                    }
                }
                36 => {
                    app.input_cursor = 0;
                    app.input_selected = false;
                }
                35 => {
                    app.input_cursor = buffer.len();
                    app.input_selected = false;
                }
                37 => {
                    app.input_cursor = buffer[..app.input_cursor]
                        .char_indices()
                        .last()
                        .map(|p| p.0)
                        .unwrap_or(0);
                    app.input_selected = false;
                }
                39 => {
                    app.input_cursor = (app.input_cursor
                        + buffer[app.input_cursor..]
                            .chars()
                            .next()
                            .map_or(0, char::len_utf8))
                    .min(buffer.len());
                    app.input_selected = false;
                }
                13 => {
                    if let Err(error) = crate::editor::commit(&mut app.model) {
                        app.model.notice = error;
                    }
                }
                _ => {}
            }
            return;
        }
        if app.model.workspace && matches!(key, 49..=51) {
            action(
                hwnd,
                app,
                match key {
                    49 => 295,
                    50 => 286,
                    _ => 287,
                },
            );
            return;
        }
        if app.model.editing.is_none()
            && let Some(handle) = app
                .model
                .focus
                .and_then(crate::viewport_edit::Handle::from_id)
            && matches!(key, 37..=40)
        {
            if crate::viewport_edit::can_edit(&app.model)
                && let Some(actor) = crate::viewport_edit::Actor::selected(&app.model)
            {
                let tool = match handle {
                    crate::viewport_edit::Handle::Heading => crate::viewport_edit::Tool::Rotate,
                    crate::viewport_edit::Handle::Uniform => crate::viewport_edit::Tool::Size,
                    _ => crate::viewport_edit::Tool::Move,
                };
                let mut amount = app.model.transform_snap.step(tool);
                if amount == 0. {
                    amount = match tool {
                        crate::viewport_edit::Tool::Move => 0.1,
                        crate::viewport_edit::Tool::Rotate => 5.,
                        crate::viewport_edit::Tool::Size => 0.05,
                    };
                }
                if matches!(key, 37 | 40) {
                    amount = -amount;
                }
                if handle == crate::viewport_edit::Handle::Uniform {
                    amount += 1.;
                }
                let result = crate::viewport_edit::transformed(
                    &app.model.document.as_ref().unwrap().scene,
                    actor,
                    handle,
                    amount,
                )
                .and_then(|s| crate::editor::edit(&mut app.model, s, true));
                if let Err(error) = result {
                    app.model.notice = error;
                }
                update_viewport(hwnd, app);
            } else {
                app.model.notice = crate::viewport_edit::unavailable_reason(&app.model).into();
            }
            return;
        }
        if key == 70
            && app.model.workspace
            && crate::viewport_edit::Actor::selected(&app.model).is_some()
        {
            if let Err(error) = frame_actor(app) {
                app.model.notice = error;
            }
            update_viewport(hwnd, app);
            return;
        }
        if key == 13 || key == 32 {
            if let Some(id) = app.model.focus {
                action(hwnd, app, id);
            }
            return;
        }
        if app.model.workspace
            && matches!(key, 87 | 65 | 83 | 68 | 81 | 69 | 70)
            && let Err(error) =
                crate::editor::camera(&mut app.model, key, if shift { 5.0 } else { 1.0 })
        {
            app.model.notice = error;
        }
    }
}
unsafe fn pause(hwnd: HWND, app: &mut App) {
    unsafe {
        KillTimer(hwnd, 2);
    }
    if let Some(session) = app.model.session.as_mut() {
        session.running = false;
    }
}
fn open_document(app: &mut App, path: &str) {
    match crate::project::Document::open(Path::new(path)) {
        Ok(document) => crate::editor::install(&mut app.model, document),
        Err(error) => app.model.notice = error,
    }
}
unsafe fn save_document(hwnd: HWND, app: &mut App, save_as: bool) -> bool {
    unsafe {
        if let Err(error) = crate::editor::commit(&mut app.model) {
            app.model.notice = error;
            return false;
        }
        let path = if save_as {
            None
        } else {
            app.model.document.as_ref().and_then(|d| d.path.clone())
        };
        let path = path.or_else(|| pick_project(hwnd, true).map(std::path::PathBuf::from));
        let Some(path) = path else {
            return false;
        };
        match crate::editor::save(&mut app.model, &path) {
            Ok(()) => true,
            Err(error) => {
                app.model.notice = error.clone();
                MessageBoxW(
                    hwnd,
                    wide(&error).as_ptr(),
                    wide("Unable to save project").as_ptr(),
                    MB_OK | MB_ICONERROR,
                );
                false
            }
        }
    }
}
unsafe fn confirm_replace(hwnd: HWND, app: &mut App) -> bool {
    unsafe {
        if let Err(error) = crate::editor::commit(&mut app.model) {
            app.model.notice = error;
            return false;
        }
        if !app.model.document.as_ref().is_some_and(|d| d.dirty()) {
            return true;
        }
        pause(hwnd, app);
        let choice=MessageBoxW(hwnd,wide("Save scene changes before continuing?\nYes: save. No: discard. Cancel: keep working.").as_ptr(),wide("Unsaved changes").as_ptr(),MB_YESNOCANCEL|MB_ICONQUESTION);
        if choice == IDYES {
            return save_document(hwnd, app, false);
        }
        if choice == IDNO {
            let recovery = crate::project::data_dir().join("recovery.json");
            let _ = std::fs::remove_file(recovery);
            app.model.recovery_available = false;
            return true;
        }
        false
    }
}

unsafe fn pick_folder(hwnd: HWND) -> Option<String> {
    unsafe {
        use windows_sys::Win32::UI::Shell::*;
        let title = wide("Choose a parent folder for your Pelagic project");
        let browse = BROWSEINFOW {
            hwndOwner: hwnd,
            lpszTitle: title.as_ptr(),
            ulFlags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE,
            ..zeroed()
        };
        let item = SHBrowseForFolderW(&browse);
        if item.is_null() {
            return None;
        }
        let mut path = vec![0_u16; 32768];
        let success = SHGetPathFromIDListEx(item, path.as_mut_ptr(), path.len() as u32, 0);
        CoTaskMemFree(item.cast());
        if success == 0 {
            None
        } else {
            let n = path.iter().position(|c| *c == 0).unwrap_or(path.len());
            Some(String::from_utf16_lossy(&path[..n]))
        }
    }
}
unsafe fn pick_project(hwnd: HWND, save: bool) -> Option<String> {
    unsafe {
        let mut name = vec![0_u16; 32768];
        if save {
            let initial = wide("Untitled ocean.pelagic");
            name[..initial.len()].copy_from_slice(&initial);
        }
        let filter = wide("Pelagic project (*.pelagic)\0*.pelagic\0");
        let extension = wide("pelagic");
        let title = wide(if save {
            "Save Pelagic project"
        } else {
            "Open a Pelagic project"
        });
        let mut dialog = OPENFILENAMEW {
            lStructSize: size_of::<OPENFILENAMEW>() as u32,
            hwndOwner: hwnd,
            lpstrFile: name.as_mut_ptr(),
            nMaxFile: name.len() as u32,
            lpstrFilter: filter.as_ptr(),
            lpstrDefExt: extension.as_ptr(),
            lpstrTitle: title.as_ptr(),
            Flags: OFN_EXPLORER
                | OFN_NOCHANGEDIR
                | OFN_PATHMUSTEXIST
                | if save {
                    OFN_OVERWRITEPROMPT
                } else {
                    OFN_FILEMUSTEXIST
                },
            ..zeroed()
        };
        let result = if save {
            GetSaveFileNameW(&mut dialog)
        } else {
            GetOpenFileNameW(&mut dialog)
        };
        if result == 0 {
            None
        } else {
            let n = name.iter().position(|x| *x == 0).unwrap_or(name.len());
            Some(String::from_utf16_lossy(&name[..n]))
        }
    }
}
