//! An independent host loads the actual exported Windows DLL at runtime.
#[cfg(windows)]
fn main() {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryW(path: *const u16) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
        fn FreeLibrary(module: *mut c_void) -> i32;
    }
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "target/release/pelagic.dll".into());
    let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let module = LoadLibraryW(wide.as_ptr());
        assert!(!module.is_null(), "Build the release DLL first");
        let version: extern "C" fn() -> u32 = std::mem::transmute(GetProcAddress(
            module,
            c"pelagic_abi_version".as_ptr().cast(),
        ));
        let create: extern "C" fn(u64) -> u64 =
            std::mem::transmute(GetProcAddress(module, c"pelagic_create".as_ptr().cast()));
        let step: extern "C" fn(u64, u32) -> i32 =
            std::mem::transmute(GetProcAddress(module, c"pelagic_step".as_ptr().cast()));
        let state: extern "C" fn(u64, u64) -> pelagic::ffi::State =
            std::mem::transmute(GetProcAddress(module, c"pelagic_state".as_ptr().cast()));
        let destroy: extern "C" fn(u64) -> i32 =
            std::mem::transmute(GetProcAddress(module, c"pelagic_destroy".as_ptr().cast()));
        assert_eq!(version(), 1);
        let handle = create(42);
        assert_ne!(handle, 0);
        assert_eq!(step(handle, 60), 1);
        let pose = state(handle, 0);
        assert_eq!(pose.valid, 1);
        assert!((pose.time - 1.0).abs() < 1e-10);
        assert_eq!(destroy(handle), 1);
        assert_eq!(state(handle, 0).valid, 0);
        println!(
            "Loaded DLL ABI v1; 60 steps; t={}s; position=({:.3},{:.3},{:.3}); released handle",
            pose.time, pose.x, pose.y, pose.z
        );
        FreeLibrary(module);
    }
}
#[cfg(not(windows))]
fn main() {
    eprintln!(
        "This dynamic-host example currently uses Windows LoadLibraryW; the C ABI is platform-neutral."
    );
}
