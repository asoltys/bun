//! Resolves the imports of a native addon before its initializer runs and calls them: with glibc binds those of zlib to Bun's zlib, and everywhere finds one that nothing defines.

use core::ffi::{CStr, c_void};
use core::slice;

use bun_sys::elf::find_loaded_module;

/// Returns 0 once every import that the loader has left unbound in the library of `initializer` has a definition, 1 after writing the name of one that has none to `name_out`, and -1 if the library's tables cannot be read.
#[unsafe(no_mangle)]
pub(crate) unsafe extern "C" fn Bun__addonResolveImports(
    initializer: *const c_void,
    handle: *mut c_void,
    name_out: *mut u8,
    name_cap: usize,
) -> i32 {
    let Some(module) = find_loaded_module(initializer.addr()) else {
        return -1;
    };
    // SAFETY: the caller holds `handle`, so the loader keeps the library of `initializer` and its program headers mapped.
    let headers =
        unsafe { slice::from_raw_parts(module.program_headers, module.program_header_count) };
    let defines = |scope: *mut c_void, name: &CStr, version: Option<&CStr>| {
        let definition = match version {
            // SAFETY: `name` is NUL-terminated, and `scope` is an open dlopen() handle or `RTLD_DEFAULT`.
            None => unsafe { libc::dlsym(scope, name.as_ptr()) },
            // SAFETY: as for dlsym(), and `version` is NUL-terminated.
            #[cfg(not(target_env = "musl"))]
            Some(version) => unsafe { libc::dlvsym(scope, name.as_ptr(), version.as_ptr()) },
            // musl has no dlvsym(): its loader binds an import by the name alone.
            #[cfg(target_env = "musl")]
            Some(_) => core::ptr::null_mut(),
        };
        !definition.is_null()
    };
    let in_own_tree = |name: &CStr, version: Option<&CStr>| defines(handle, name, version);
    let is_defined = |name: &CStr, version: Option<&CStr>| {
        defines(libc::RTLD_DEFAULT, name, version) || in_own_tree(name, version)
    };
    // musl binds a pending import again at every later dlopen(), which would undo a stored slot, and nothing has run this on FreeBSD.
    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    // SAFETY: `headers` are those of the library that `handle` keeps loaded, and `base_address` is where the loader mapped it.
    let unresolved = unsafe {
        bun_exe_format::elf_imports::unresolved_lazy_import(
            headers,
            module.base_address,
            // musl leaves a `GLOB_DAT` import that nothing defines unbound as it does a jump slot. FreeBSD binds it in dlopen().
            cfg!(target_env = "musl"),
            is_defined,
        )
    };
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: `headers` are those of the library that `handle` keeps loaded, and `base_address` is where the loader mapped it.
    let unresolved = unsafe {
        host_zlib::bind(
            headers,
            module.base_address,
            |name| in_own_tree(name, None),
            is_defined,
        )
    };
    match unresolved {
        Ok(None) => 0,
        Ok(Some(name)) => {
            // SAFETY: the caller gives `name_cap` writable bytes, and at least one.
            let out = unsafe { slice::from_raw_parts_mut(name_out, name_cap) };
            let name = name.to_bytes();
            let length = name.len().min(out.len() - 1);
            out[..length].copy_from_slice(&name[..length]);
            out[length] = 0;
            1
        }
        Err(_) => -1,
    }
}

/// Node's executable exports its zlib to addons. Bun's zlib is not exported, so that no other library of the process binds to it.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
mod host_zlib {
    use core::ffi::CStr;

    use bun_exe_format::elf_imports::{HostLibrary, Imports, Unreadable, resolve_lazy_imports};
    use bun_zlib as zlib;

    /// The 89 functions that `vendor/zlib/zlib.h.in` declares, without `gzopen_w` (Windows): what a system's libz.so.1 and Node export.
    static API: [&CStr; 88] = [
        c"adler32",
        c"adler32_combine",
        c"adler32_combine64",
        c"adler32_z",
        c"compress",
        c"compress2",
        c"compressBound",
        c"crc32",
        c"crc32_combine",
        c"crc32_combine64",
        c"crc32_combine_gen",
        c"crc32_combine_gen64",
        c"crc32_combine_op",
        c"crc32_z",
        c"deflate",
        c"deflateBound",
        c"deflateCopy",
        c"deflateEnd",
        c"deflateGetDictionary",
        c"deflateInit2_",
        c"deflateInit_",
        c"deflateParams",
        c"deflatePending",
        c"deflatePrime",
        c"deflateReset",
        c"deflateResetKeep",
        c"deflateSetDictionary",
        c"deflateSetHeader",
        c"deflateTune",
        c"get_crc_table",
        c"gzbuffer",
        c"gzclearerr",
        c"gzclose",
        c"gzclose_r",
        c"gzclose_w",
        c"gzdirect",
        c"gzdopen",
        c"gzeof",
        c"gzerror",
        c"gzflush",
        c"gzfread",
        c"gzfwrite",
        c"gzgetc",
        c"gzgetc_",
        c"gzgets",
        c"gzoffset",
        c"gzoffset64",
        c"gzopen",
        c"gzopen64",
        c"gzprintf",
        c"gzputc",
        c"gzputs",
        c"gzread",
        c"gzrewind",
        c"gzseek",
        c"gzseek64",
        c"gzsetparams",
        c"gztell",
        c"gztell64",
        c"gzungetc",
        c"gzvprintf",
        c"gzwrite",
        c"inflate",
        c"inflateBack",
        c"inflateBackEnd",
        c"inflateBackInit_",
        c"inflateCodesUsed",
        c"inflateCopy",
        c"inflateEnd",
        c"inflateGetDictionary",
        c"inflateGetHeader",
        c"inflateInit2_",
        c"inflateInit_",
        c"inflateMark",
        c"inflatePrime",
        c"inflateReset",
        c"inflateReset2",
        c"inflateResetKeep",
        c"inflateSetDictionary",
        c"inflateSync",
        c"inflateSyncPoint",
        c"inflateUndermine",
        c"inflateValidate",
        c"uncompress",
        c"uncompress2",
        c"zError",
        c"zlibCompileFlags",
        c"zlibVersion",
    ];

    /// The imports of uWebSockets.js. Each function here stays in the binary for addons alone, if nothing else calls it.
    fn functions() -> [(&'static CStr, usize); 13] {
        [
            (c"deflate", zlib::deflate as *const () as usize),
            (c"deflateBound", zlib::deflateBound as *const () as usize),
            (c"deflateEnd", zlib::deflateEnd as *const () as usize),
            (c"deflateInit2_", zlib::deflateInit2_ as *const () as usize),
            (c"deflateInit_", zlib::deflateInit_ as *const () as usize),
            (c"deflateReset", zlib::deflateReset as *const () as usize),
            (
                c"deflateSetDictionary",
                zlib::deflateSetDictionary as *const () as usize,
            ),
            (c"inflate", zlib::inflate as *const () as usize),
            (c"inflateEnd", zlib::inflateEnd as *const () as usize),
            (c"inflateInit2_", zlib::inflateInit2_ as *const () as usize),
            (c"inflateInit_", zlib::inflateInit_ as *const () as usize),
            (c"inflateReset", zlib::inflateReset as *const () as usize),
            (
                c"inflateSetDictionary",
                zlib::inflateSetDictionary as *const () as usize,
            ),
        ]
    }

    /// Stores Bun's zlib functions in the slots of an addon that imports zlib and has none in its own tree, and returns the first import that nothing defines.
    pub(super) unsafe fn bind<'a>(
        headers: &'a [libc::Elf64_Phdr],
        base: usize,
        in_own_tree: impl FnMut(&CStr) -> bool,
        is_defined: impl FnMut(&CStr, Option<&CStr>) -> bool,
    ) -> Result<Option<&'a CStr>, Unreadable> {
        let library = HostLibrary {
            api: &API,
            functions: &functions(),
        };
        // SAFETY: the caller keeps the object of `headers` loaded at `base`.
        match unsafe { resolve_lazy_imports(headers, base, &library, in_own_tree, is_defined) }? {
            Imports::Unresolved(name) => Ok(Some(name)),
            Imports::Resolved(bindings) => {
                for binding in bindings {
                    // SAFETY: the caller keeps the object loaded, and each slot is one that `resolve_lazy_imports` checked in it.
                    unsafe { binding.store() };
                }
                Ok(None)
            }
        }
    }
}
