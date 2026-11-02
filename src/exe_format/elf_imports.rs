//! The imports that the dynamic loader has left unbound in an ELF object that it has mapped into this process.

use core::ffi::CStr;
use core::mem::{align_of, size_of};
use core::slice;
use core::sync::atomic::{AtomicUsize, Ordering};

use bun_core::AnyBitPattern;
use bun_sys::elf::PT_LOAD;

const PT_DYNAMIC: u32 = 2;
const PT_GNU_RELRO: u32 = 0x6474_e552;
const PF_W: u32 = 2;
const PF_R: u32 = 4;
const DT_NULL: i64 = 0;
const DT_PLTRELSZ: i64 = 2;
const DT_STRTAB: i64 = 5;
const DT_SYMTAB: i64 = 6;
const DT_RELA: i64 = 7;
const DT_RELASZ: i64 = 8;
const DT_RELAENT: i64 = 9;
const DT_STRSZ: i64 = 10;
const DT_PLTREL: i64 = 20;
const DT_JMPREL: i64 = 23;
const DT_BIND_NOW: i64 = 24;
const DT_FLAGS: i64 = 30;
const DT_VERSYM: i64 = 0x6fff_fff0;
const DT_FLAGS_1: i64 = 0x6fff_fffb;
const DT_VERNEED: i64 = 0x6fff_fffe;
const DT_VERNEEDNUM: i64 = 0x6fff_ffff;
const DF_BIND_NOW: u64 = 8;
const DF_1_NOW: u64 = 1;
const RELA_SIZE: u64 = size_of::<Elf64_Rela>() as u64;
const SHN_UNDEF: u16 = 0;
const STB_GLOBAL: u8 = 1;
/// The low 15 bits of a `DT_VERSYM` element and of `vna_other`: 0 and 1 mean that the symbol has no version.
const VERSION_INDEX: u16 = 0x7fff;
#[cfg(target_arch = "x86_64")]
const R_GLOB_DAT: u32 = 6;
#[cfg(target_arch = "x86_64")]
const R_JUMP_SLOT: u32 = 7;
#[cfg(target_arch = "aarch64")]
const R_GLOB_DAT: u32 = 1025;
#[cfg(target_arch = "aarch64")]
const R_JUMP_SLOT: u32 = 1026;

#[repr(C)]
#[derive(Clone, Copy)]
struct Elf64_Dyn {
    d_tag: i64,
    d_val: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Elf64_Rela {
    r_offset: u64,
    r_info: u64,
    _r_addend: i64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Elf64_Sym {
    st_name: u32,
    st_info: u8,
    _st_other: u8,
    st_shndx: u16,
    _st_value: u64,
    _st_size: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Elf64_Verneed {
    _vn_version: u16,
    vn_cnt: u16,
    _vn_file: u32,
    vn_aux: u32,
    vn_next: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Elf64_Vernaux {
    _vna_hash: u32,
    _vna_flags: u16,
    vna_other: u16,
    vna_name: u32,
    vna_next: u32,
}

bun_core::impl_any_bit_pattern!(
    Elf64_Dyn,
    Elf64_Rela,
    Elf64_Sym,
    Elf64_Verneed,
    Elf64_Vernaux
);

/// Why the imports of an object cannot be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unreadable {
    /// There is no `PT_DYNAMIC` segment, or it has no `DT_NULL` inside its size and inside a `PT_LOAD` range.
    Dynamic,
    /// A relocation table, `DT_SYMTAB` or `DT_STRTAB` points outside the `PT_LOAD` ranges, or the last two are missing.
    Table,
    /// A relocation table is larger than the `PT_LOAD` range around it, or `DT_PLTREL` or `DT_RELAENT` names another format than `Elf64_Rela`.
    Relocations,
    /// A relocation names a symbol outside the `PT_LOAD` range of the symbol table.
    SymbolIndex,
    /// The name of a symbol does not end inside `DT_STRSZ`.
    SymbolName,
    /// The slot of an import that the host binds is not a pointer inside the ranges that the loader leaves writable.
    Slot,
}

/// A table of the object: `room` is the bytes from `address` to the end of the table, or of the `PT_LOAD` range around it.
#[derive(Clone, Copy)]
struct Table {
    address: usize,
    room: usize,
}

impl Table {
    /// The `T` that starts `offset` bytes into the table.
    fn at<T: AnyBitPattern>(self, offset: usize) -> Option<T> {
        if offset.checked_add(size_of::<T>())? > self.room {
            return None;
        }
        // SAFETY: the element is inside a range that the loader mapped readable, and the caller of the walk keeps the object loaded.
        Some(unsafe { (self.address.wrapping_add(offset) as *const T).read_unaligned() })
    }

    fn element<T: AnyBitPattern>(self, index: usize) -> Option<T> {
        self.at(index.checked_mul(size_of::<T>())?)
    }

    fn string<'a>(self, offset: usize) -> Option<&'a CStr> {
        let length = self.room.checked_sub(offset)?;
        // SAFETY: as in `element`, and nothing writes to the string table of a loaded object.
        let bytes = unsafe {
            slice::from_raw_parts(self.address.wrapping_add(offset) as *const u8, length)
        };
        CStr::from_bytes_until_nul(bytes).ok()
    }
}

struct Image<'a> {
    headers: &'a [libc::Elf64_Phdr],
    base: usize,
}

impl Image<'_> {
    /// The bytes from `address` to the end of the range around it, of the `kind` headers that have `flags`.
    fn room(&self, kind: u32, flags: u32, address: usize) -> usize {
        self.headers
            .iter()
            .filter(|header| header.p_type == kind && header.p_flags & flags == flags)
            .find_map(|header| {
                let start = self.base.wrapping_add(header.p_vaddr as usize);
                (header.p_memsz as usize)
                    .checked_sub(address.wrapping_sub(start))
                    .filter(|&room| room != 0)
            })
            .unwrap_or(0)
    }

    /// glibc stores the address of a table in the dynamic entry, musl and FreeBSD leave its offset from the base there.
    fn table(&self, pointer: u64) -> Option<Table> {
        [pointer as usize, self.base.wrapping_add(pointer as usize)]
            .into_iter()
            .map(|address| Table {
                address,
                room: self.room(PT_LOAD, PF_R, address),
            })
            .find(|table| table.room != 0)
    }

    /// The loader makes `PT_GNU_RELRO` read-only once it has relocated the object.
    fn has_writable_pointer(&self, address: usize) -> bool {
        address.is_multiple_of(align_of::<usize>())
            && self.room(PT_LOAD, PF_R | PF_W, address) >= size_of::<usize>()
            && self.room(PT_GNU_RELRO, 0, address) == 0
    }

    /// The version that the object names for its import `symbol`. A version table that is missing or malformed names none.
    fn version<'a>(&self, versions: Versions, strings: Table, symbol: usize) -> Option<&'a CStr> {
        let index = self.table(versions.symbols?)?.element::<u16>(symbol)? & VERSION_INDEX;
        if index < 2 {
            return None;
        }
        let needed = self.table(versions.needed?)?;
        // A version index has 15 bits, so tables that are well formed have fewer entries than this, and a longer chain ends here.
        let mut budget = 0..1u32 << 16;
        let mut need_offset = 0usize;
        for _ in 0..versions.needed_count {
            budget.next()?;
            let need: Elf64_Verneed = needed.at(need_offset)?;
            let mut aux_offset = need_offset.checked_add(need.vn_aux as usize)?;
            for _ in 0..need.vn_cnt {
                budget.next()?;
                let aux: Elf64_Vernaux = needed.at(aux_offset)?;
                if aux.vna_other & VERSION_INDEX == index {
                    return strings.string(aux.vna_name as usize);
                }
                if aux.vna_next == 0 {
                    break;
                }
                aux_offset = aux_offset.checked_add(aux.vna_next as usize)?;
            }
            if need.vn_next == 0 {
                break;
            }
            need_offset = need_offset.checked_add(need.vn_next as usize)?;
        }
        None
    }
}

/// The values of `DT_VERSYM`, `DT_VERNEED` and `DT_VERNEEDNUM`. The walk reads these tables only for an import that has no definition by its name.
#[derive(Clone, Copy, Default)]
struct Versions {
    symbols: Option<u64>,
    needed: Option<u64>,
    needed_count: u64,
}

/// Hands the name and the slot of each import that the loader has left unbound to `takes`, and returns the first that it does not take and that `is_defined` rejects by its name and then by the version that the object names for it. The loader keeps `headers` mapped at `base`, and `defers_got` is for musl, which leaves a `GLOB_DAT` import unbound too.
unsafe fn each_lazy_import<'a>(
    headers: &'a [libc::Elf64_Phdr],
    base: usize,
    defers_got: bool,
    mut takes: impl FnMut(&'a CStr, usize) -> bool,
    mut is_defined: impl FnMut(&CStr, Option<&CStr>) -> bool,
) -> Result<Option<&'a CStr>, Unreadable> {
    let image = Image { headers, base };
    let dynamic = headers
        .iter()
        .find(|header| header.p_type == PT_DYNAMIC)
        .ok_or(Unreadable::Dynamic)?;
    let address = base.wrapping_add(dynamic.p_vaddr as usize);
    let entries = Table {
        address,
        room: image
            .room(PT_LOAD, PF_R, address)
            .min(dynamic.p_memsz as usize),
    };

    // `rela_plt` is the table of `DT_JMPREL`, and `rela_dyn` that of `DT_RELA`, which is read only for musl.
    let (mut rela_plt, mut rela_dyn, mut symbols, mut strings) = (None, None, None, None);
    let (mut rela_plt_size, mut rela_dyn_size, mut strings_size) = (0, 0, 0);
    let (mut bound, mut other_format, mut versions) = (false, false, Versions::default());
    for index in 0.. {
        let entry: Elf64_Dyn = entries.element(index).ok_or(Unreadable::Dynamic)?;
        let table = || Some(image.table(entry.d_val).ok_or(Unreadable::Table));
        match entry.d_tag {
            DT_NULL => break,
            DT_JMPREL => rela_plt = table(),
            DT_RELA if defers_got => rela_dyn = table(),
            DT_SYMTAB => symbols = table(),
            DT_STRTAB => strings = table(),
            DT_PLTRELSZ => rela_plt_size = entry.d_val as usize,
            DT_RELASZ => rela_dyn_size = entry.d_val as usize,
            DT_STRSZ => strings_size = entry.d_val as usize,
            DT_PLTREL => other_format |= entry.d_val != DT_RELA as u64,
            DT_RELAENT if defers_got => other_format |= entry.d_val != RELA_SIZE,
            DT_BIND_NOW => bound = true,
            DT_FLAGS => bound |= entry.d_val & DF_BIND_NOW != 0,
            DT_FLAGS_1 => bound |= entry.d_val & DF_1_NOW != 0,
            DT_VERSYM => versions.symbols = Some(entry.d_val),
            DT_VERNEED => versions.needed = Some(entry.d_val),
            DT_VERNEEDNUM => versions.needed_count = entry.d_val,
            _ => {}
        }
    }
    // The loader bound every import of such an object inside dlopen(), which fails when nothing defines one. Only a glibc that profiles or audits calls (LD_PROFILE, LD_AUDIT) does not.
    if bound {
        return Ok(None);
    }
    if other_format {
        return Err(Unreadable::Relocations);
    }
    let sized = |table: Option<Result<Table, Unreadable>>, size| match table.transpose()? {
        Some(table) if size > table.room => Err(Unreadable::Relocations),
        table => Ok(table.map(|table| Table {
            room: size,
            ..table
        })),
    };
    let (rela_plt, rela_dyn) = (
        sized(rela_plt, rela_plt_size)?,
        sized(rela_dyn, rela_dyn_size)?,
    );
    let (symbols, strings) = (symbols.transpose()?, strings.transpose()?);
    if rela_plt.is_none() && rela_dyn.is_none() {
        return Ok(None);
    }
    let (Some(symbols), Some(strings)) = (symbols, strings) else {
        return Err(Unreadable::Table);
    };
    let strings = Table {
        room: strings.room.min(strings_size),
        ..strings
    };

    let relocations = [rela_plt, rela_dyn]
        .into_iter()
        .flatten()
        .flat_map(|table| (0..).map_while(move |index| table.element::<Elf64_Rela>(index)));
    for relocation in relocations {
        let kind = relocation.r_info as u32;
        if kind != R_JUMP_SLOT && !(defers_got && kind == R_GLOB_DAT) {
            continue;
        }
        let index = (relocation.r_info >> 32) as usize;
        let symbol: Elf64_Sym = symbols.element(index).ok_or(Unreadable::SymbolIndex)?;
        // A weak import may stay unresolved: the loader binds it to address 0.
        if symbol.st_info >> 4 != STB_GLOBAL || symbol.st_shndx != SHN_UNDEF {
            continue;
        }
        let name = strings
            .string(symbol.st_name as usize)
            .ok_or(Unreadable::SymbolName)?;
        let slot = base.wrapping_add(relocation.r_offset as usize);
        if takes(name, slot) || is_defined(name, None) {
            continue;
        }
        // The loader binds by the version that the object names, and a lookup by the name alone misses a definition that is kept only under an old version.
        let version = image.version(versions, strings, index);
        if !version.is_some_and(|version| is_defined(name, Some(version))) {
            return Ok(Some(name));
        }
    }
    Ok(None)
}

/// The first import that the loader has left unbound and that `is_defined` rejects by its name and then by the version that the object names for it, of an object whose loader keeps `headers` mapped at `base`. `defers_got` is for musl, which leaves a `GLOB_DAT` import unbound too.
pub unsafe fn unresolved_lazy_import<'a>(
    headers: &'a [libc::Elf64_Phdr],
    base: usize,
    defers_got: bool,
    is_defined: impl FnMut(&CStr, Option<&CStr>) -> bool,
) -> Result<Option<&'a CStr>, Unreadable> {
    // SAFETY: the caller keeps the object of `headers` loaded at `base`.
    unsafe { each_lazy_import(headers, base, defers_got, |_, _| false, is_defined) }
}

/// A library that the host has inside it and does not export, whose functions an addon can import from the host.
pub struct HostLibrary<'library> {
    /// The name of every function that the library exports.
    pub api: &'library [&'library CStr],
    /// The functions that the host binds, with their addresses.
    pub functions: &'library [(&'library CStr, usize)],
}

/// A function of the host, and the slot through which a loaded object calls its import of that name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    slot: usize,
    function: usize,
}

impl Binding {
    /// Makes the object call the host's function. The object that `resolve_lazy_imports` read must still be loaded.
    pub unsafe fn store(self) {
        // SAFETY: `resolve_lazy_imports` found the slot aligned and inside a range that the loader keeps writable.
        let slot = unsafe { AtomicUsize::from_ptr(self.slot as *mut usize) };
        // Loads of one library in several Workers store the same value here at the same time, while other threads call through the slot.
        if slot.load(Ordering::SeqCst) != self.function {
            slot.store(self.function, Ordering::SeqCst);
        }
    }
}

/// Where the lazily bound function imports of an object resolve.
#[derive(Debug, PartialEq, Eq)]
pub enum Imports<'a> {
    /// Every import has a definition, once each of these is stored.
    Resolved(Vec<Binding>),
    /// Nothing that the object may bind to defines this import.
    Unresolved(&'a CStr),
}

/// Decides where the lazily bound function imports of an object resolve, and stores nothing: a function of `library` in the object's own tree (`in_own_tree`) or in the host, any other where `is_defined` finds it by its name or by the version that the object names for it.
pub unsafe fn resolve_lazy_imports<'a>(
    headers: &'a [libc::Elf64_Phdr],
    base: usize,
    library: &HostLibrary<'_>,
    mut in_own_tree: impl FnMut(&CStr) -> bool,
    is_defined: impl FnMut(&CStr, Option<&CStr>) -> bool,
) -> Result<Imports<'a>, Unreadable> {
    let image = Image { headers, base };
    let mut bindings = Vec::new();
    let (mut own, mut not_own, mut not_bound, mut writable) = (false, None, None, true);
    // A function of the library is decided by its name alone, whatever version the object names for it: the host's functions have no version.
    let of_library = |name, slot| {
        if !library.api.contains(&name) {
            return false;
        }
        if in_own_tree(name) {
            own = true;
        } else if not_own.is_none() {
            not_own = Some(name);
        }
        match library.functions.iter().find(|(bound, _)| *bound == name) {
            Some(&(_, function)) => {
                writable &= image.has_writable_pointer(slot);
                bindings.push(Binding { slot, function });
            }
            None if not_bound.is_none() => not_bound = Some(name),
            None => {}
        }
        true
    };
    // Only glibc gets a host library, and glibc binds a `GLOB_DAT` import itself in dlopen().
    let defers_got = false;
    // SAFETY: the caller keeps the object of `headers` loaded at `base`.
    let undefined = unsafe { each_lazy_import(headers, base, defers_got, of_library, is_defined) }?;
    // One stream must not pass through two implementations of the library: its functions all come from the object's own tree, or all from the host.
    if let Some(name) = undefined.or(if own { not_own } else { not_bound }) {
        return Ok(Imports::Unresolved(name));
    }
    if own {
        return Ok(Imports::Resolved(Vec::new()));
    }
    if !writable {
        return Err(Unreadable::Slot);
    }
    Ok(Imports::Resolved(bindings))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT_REL: u64 = 17;
    const DT_DEBUG: i64 = 21;
    const STB_WEAK: u8 = 2;
    const DYNAMIC_ENTRY: usize = 16;
    const SYMBOL: usize = 24;
    const RELOCATION: usize = 24;
    const VERSION_ENTRY: usize = 16;
    const SLOT: usize = size_of::<usize>();
    const GAP: usize = 11 * SYMBOL;
    /// The last but one is a spare entry, which `add_dynamic` replaces.
    const DYNAMIC_TAGS: [i64; 14] = [
        DT_JMPREL,
        DT_PLTRELSZ,
        DT_PLTREL,
        DT_SYMTAB,
        DT_STRTAB,
        DT_STRSZ,
        DT_RELA,
        DT_RELASZ,
        DT_RELAENT,
        DT_VERSYM,
        DT_VERNEED,
        DT_VERNEEDNUM,
        DT_DEBUG,
        DT_NULL,
    ];
    /// The names of the version indexes 2 to 5, which every `Object` has at the start of its string table.
    const VERSIONS: [&str; 4] = ["VERSION_2", "VERSION_3", "VERSION_4", "VERSION_5"];
    /// The offsets of the fields of an `Elf64_Verneed` and of an `Elf64_Vernaux`.
    const VN_CNT: usize = 2;
    const VN_AUX: usize = 8;
    const VN_NEXT: usize = 12;
    const VNA_OTHER: usize = 6;
    const VNA_NAME: usize = 8;
    const VNA_NEXT: usize = 12;

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Section {
        RelaPlt,
        RelaDyn,
    }

    #[derive(Clone, Copy)]
    struct Import {
        name: &'static str,
        binding: u8,
        section: u16,
        relocation_type: u32,
        /// The table that has the relocation: that of `DT_JMPREL`, or that of `DT_RELA`.
        relocated_in: Section,
        /// The element of `DT_VERSYM` for the symbol: 1 is a symbol without a version.
        version: u16,
    }

    const fn import(name: &'static str) -> Import {
        Import {
            name,
            binding: STB_GLOBAL,
            section: SHN_UNDEF,
            relocation_type: R_JUMP_SLOT,
            relocated_in: Section::RelaPlt,
            version: 1,
        }
    }

    /// An import that names the version of index `version`.
    const fn versioned(name: &'static str, version: u16) -> Import {
        Import {
            version,
            ..import(name)
        }
    }

    /// An import through the global offset table, as `-fno-plt` compiles a call.
    const fn got_import(name: &'static str) -> Import {
        Import {
            relocation_type: R_GLOB_DAT,
            relocated_in: Section::RelaDyn,
            ..import(name)
        }
    }

    /// The offset of the name of a version index in the string table of an `Object`. The indexes below 2 get the first name.
    fn version_name(index: u16) -> u32 {
        let before = &VERSIONS[..usize::from(index).saturating_sub(2)];
        1 + before.iter().map(|name| name.len() as u32 + 1).sum::<u32>()
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Pointers {
        Addresses,
        Offsets,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Linked {
        AtZero,
        AboveItsMapping,
    }

    /// An object in this process's memory: the symbols, strings and relocations of `DT_JMPREL` in one segment, and after gaps the dynamic section in another, the version tables and the relocations of `DT_RELA` in a third, and the slots of the imports in a fourth.
    struct Object {
        memory: Vec<u8>,
        link_address: u64,
        tables_flags: u32,
        tables_size: usize,
        strings_size: usize,
        relocations: usize,
        dynamic: usize,
        dynamic_load_size: usize,
        dynamic_size: Option<usize>,
        /// The third segment, which starts with the `DT_VERSYM` array.
        versions: usize,
        versions_size: usize,
        /// The `DT_VERNEED` table, which is empty until `set_version_needs`.
        version_needs: usize,
        /// The `DT_RELA` table, with an entry for each import as the `DT_JMPREL` table has.
        data_relocations: usize,
        /// Whether the walk is that of musl.
        defers_got: bool,
        slots: usize,
        slots_flags: u32,
        slots_size: usize,
        read_only_after_relocation: Option<(usize, usize)>,
    }

    impl Object {
        fn new(linked: Linked, pointers: Pointers, imports: &[Import]) -> Object {
            Object::with_version_needs_room(linked, pointers, imports, 8 * VERSION_ENTRY)
        }

        fn with_version_needs_room(
            linked: Linked,
            pointers: Pointers,
            imports: &[Import],
            version_needs_room: usize,
        ) -> Object {
            let mut names = vec![0u8];
            for version in VERSIONS {
                names.extend_from_slice(version.as_bytes());
                names.push(0);
            }
            let name_offsets: Vec<usize> = imports
                .iter()
                .map(|import| {
                    names.extend_from_slice(import.name.as_bytes());
                    names.push(0);
                    names.len() - import.name.len() - 1
                })
                .collect();
            let strings = (1 + imports.len()) * SYMBOL;
            let relocations = strings + names.len().next_multiple_of(SYMBOL);
            let tables_size = relocations + imports.len() * RELOCATION;
            let dynamic = tables_size + GAP;
            let dynamic_load_size = DYNAMIC_TAGS.len() * DYNAMIC_ENTRY;
            let versions = dynamic + dynamic_load_size + GAP;
            let version_needs = versions + (2 * (1 + imports.len())).next_multiple_of(8);
            let data_relocations = version_needs + version_needs_room;
            let versions_size = data_relocations + imports.len() * RELOCATION - versions;
            let slots_size = imports.len() * SLOT;
            let memory = vec![0; versions + versions_size + GAP + SLOT + slots_size + GAP];
            let slots = versions + versions_size + GAP;
            let slots = slots + memory.as_ptr().wrapping_add(slots).align_offset(SLOT);

            let mut object = Object {
                memory,
                link_address: 0,
                tables_flags: PF_R,
                tables_size,
                strings_size: names.len(),
                relocations,
                dynamic,
                dynamic_load_size,
                dynamic_size: Some(dynamic_load_size),
                versions,
                versions_size,
                version_needs,
                data_relocations,
                defers_got: false,
                slots,
                slots_flags: PF_R | PF_W,
                slots_size,
                read_only_after_relocation: None,
            };
            if linked == Linked::AboveItsMapping {
                object.link_address = (object.memory.as_ptr() as u64).wrapping_add(0x1000_0000);
            }
            object.memory[strings..][..names.len()].copy_from_slice(&names);
            for (index, import) in imports.iter().enumerate() {
                let symbol = (1 + index) * SYMBOL;
                object.write_u32(symbol, name_offsets[index] as u32);
                object.memory[symbol + 4] = import.binding << 4;
                object.write_u16(symbol + 6, import.section);
                object.write_u16(versions + 2 * (1 + index), import.version);
                match import.relocated_in {
                    Section::RelaPlt => {
                        object.set_relocation(index, 1 + index, import.relocation_type)
                    }
                    Section::RelaDyn => {
                        object.set_data_relocation(index, 1 + index, import.relocation_type)
                    }
                }
                object.set_relocation_offset(index, slots + index * SLOT);
                object.write_u64(slots + index * SLOT, (LAZY + index) as u64);
            }
            let values = [
                object.pointer(pointers, relocations),
                (imports.len() * RELOCATION) as u64,
                DT_RELA as u64,
                object.pointer(pointers, 0),
                object.pointer(pointers, strings),
                names.len() as u64,
                object.pointer(pointers, data_relocations),
                (imports.len() * RELOCATION) as u64,
                RELOCATION as u64,
                object.pointer(pointers, versions),
                object.pointer(pointers, version_needs),
                0,
                0,
                0,
            ];
            for (index, (tag, value)) in DYNAMIC_TAGS.into_iter().zip(values).enumerate() {
                object.write_u64(dynamic + index * DYNAMIC_ENTRY, tag as u64);
                object.write_u64(dynamic + index * DYNAMIC_ENTRY + 8, value);
            }
            object
        }

        fn pointer(&self, pointers: Pointers, offset: usize) -> u64 {
            match pointers {
                Pointers::Addresses => self.memory.as_ptr() as u64 + offset as u64,
                Pointers::Offsets => self.link_address.wrapping_add(offset as u64),
            }
        }

        fn base(&self) -> usize {
            (self.memory.as_ptr() as usize).wrapping_sub(self.link_address as usize)
        }

        fn write_u16(&mut self, offset: usize, value: u16) {
            self.memory[offset..][..2].copy_from_slice(&value.to_le_bytes());
        }

        fn write_u32(&mut self, offset: usize, value: u32) {
            self.memory[offset..][..4].copy_from_slice(&value.to_le_bytes());
        }

        fn write_u64(&mut self, offset: usize, value: u64) {
            self.memory[offset..][..8].copy_from_slice(&value.to_le_bytes());
        }

        fn set_relocation(&mut self, index: usize, symbol: usize, relocation_type: u32) {
            let info = (symbol as u64) << 32 | u64::from(relocation_type);
            self.write_u64(self.relocations + index * RELOCATION + 8, info);
        }

        fn set_relocation_offset(&mut self, index: usize, offset: usize) {
            let offset = self.link_address.wrapping_add(offset as u64);
            self.write_u64(self.relocations + index * RELOCATION, offset);
        }

        fn set_data_relocation(&mut self, index: usize, symbol: usize, relocation_type: u32) {
            let info = (symbol as u64) << 32 | u64::from(relocation_type);
            self.write_u64(self.data_relocations + index * RELOCATION + 8, info);
        }

        /// Writes the `DT_VERNEED` table: an `Elf64_Verneed` for each of `needs`, each before an `Elf64_Vernaux` for each of its version indexes.
        fn set_version_needs(&mut self, needs: &[&[u16]]) {
            let mut need = self.version_needs;
            for (position, indexes) in needs.iter().enumerate() {
                let size = (1 + indexes.len()) * VERSION_ENTRY;
                let last = position + 1 == needs.len();
                self.write_u16(need + VN_CNT, indexes.len() as u16);
                self.write_u32(need + VN_AUX, VERSION_ENTRY as u32);
                self.write_u32(need + VN_NEXT, if last { 0 } else { size as u32 });
                for (position, &index) in indexes.iter().enumerate() {
                    let aux = need + (1 + position) * VERSION_ENTRY;
                    let last = position + 1 == indexes.len();
                    self.write_u16(aux + VNA_OTHER, index);
                    self.write_u32(aux + VNA_NAME, version_name(index & VERSION_INDEX));
                    self.write_u32(aux + VNA_NEXT, if last { 0 } else { VERSION_ENTRY as u32 });
                }
                need += size;
            }
            self.set_dynamic(DT_VERNEEDNUM, needs.len() as u64);
        }

        /// The offset of a field of the `entry`th `Elf64_Verneed` or `Elf64_Vernaux` of the `DT_VERNEED` table.
        fn version_field(&self, entry: usize, field: usize) -> usize {
            self.version_needs + entry * VERSION_ENTRY + field
        }

        fn set_symbol_name(&mut self, import: usize, offset: usize) {
            self.write_u32((1 + import) * SYMBOL, offset as u32);
        }

        fn dynamic_entry(&self, tag: i64) -> usize {
            self.dynamic + DYNAMIC_TAGS.iter().position(|&t| t == tag).unwrap() * DYNAMIC_ENTRY
        }

        fn set_dynamic(&mut self, tag: i64, value: u64) {
            self.write_u64(self.dynamic_entry(tag) + 8, value);
        }

        fn set_dynamic_tag(&mut self, tag: i64, new_tag: i64) {
            self.write_u64(self.dynamic_entry(tag), new_tag as u64);
        }

        fn add_dynamic(&mut self, tag: i64, value: u64) {
            self.set_dynamic(DT_DEBUG, value);
            self.set_dynamic_tag(DT_DEBUG, tag);
        }

        fn headers(&self) -> Vec<libc::Elf64_Phdr> {
            let header = |p_type, p_flags, offset: usize, size: usize| libc::Elf64_Phdr {
                p_type,
                p_flags,
                p_offset: offset as u64,
                p_vaddr: self.link_address.wrapping_add(offset as u64),
                p_paddr: 0,
                p_filesz: size as u64,
                p_memsz: size as u64,
                p_align: 1,
            };
            let mut headers = vec![
                header(PT_LOAD, self.tables_flags, 0, self.tables_size),
                header(PT_LOAD, PF_R | PF_W, self.dynamic, self.dynamic_load_size),
                header(PT_LOAD, PF_R, self.versions, self.versions_size),
                header(PT_LOAD, self.slots_flags, self.slots, self.slots_size),
            ];
            if let Some(size) = self.dynamic_size {
                headers.push(header(PT_DYNAMIC, PF_R | PF_W, self.dynamic, size));
            }
            if let Some((offset, size)) = self.read_only_after_relocation {
                headers.push(header(PT_GNU_RELRO, PF_R, offset, size));
            }
            headers
        }

        /// The result of the walk when only `defined` have a definition, and what it asked for: each a name, or a name and a version as `name@version`.
        fn walk(&self, defined: &[&str]) -> (Result<Option<String>, Unreadable>, Vec<String>) {
            let mut asked = Vec::new();
            let headers = self.headers();
            // SAFETY: the headers describe ranges inside `self.memory`, which outlives the call.
            let found = unsafe {
                unresolved_lazy_import(&headers, self.base(), self.defers_got, |name, version| {
                    asked.push(question(name, version));
                    defined.contains(&asked.last().unwrap().as_str())
                })
            };
            (
                found.map(|name| name.map(|name| name.to_str().unwrap().to_owned())),
                asked,
            )
        }

        fn slot(&self, import: usize) -> usize {
            self.memory.as_ptr() as usize + self.slots + import * SLOT
        }

        fn slot_values(&self) -> Vec<usize> {
            let (slots, _) = self.memory[self.slots..].as_chunks::<SLOT>();
            let imports = (self.tables_size - self.relocations) / RELOCATION;
            slots[..imports]
                .iter()
                .map(|slot| usize::from_le_bytes(*slot))
                .collect()
        }

        /// The decision about the imports of `ZLIB` and the others when the object's own tree defines `own` and the process defines `defined`, with what each was asked for: the process as `walk` is.
        fn resolve(&self, own: &[&str], defined: &[&str]) -> Resolution {
            let (mut asked_own_tree, mut asked_defined) = (Vec::new(), Vec::new());
            let headers = self.headers();
            // SAFETY: the headers describe ranges inside `self.memory`, which outlives the call.
            let imports = unsafe {
                resolve_lazy_imports(
                    &headers,
                    self.base(),
                    &ZLIB,
                    |name| {
                        let name = name.to_str().unwrap();
                        asked_own_tree.push(name.to_owned());
                        own.contains(&name)
                    },
                    |name, version| {
                        asked_defined.push(question(name, version));
                        defined.contains(&asked_defined.last().unwrap().as_str())
                    },
                )
            };
            let mut bindings = Vec::new();
            let decided = imports.map(|imports| match imports {
                Imports::Resolved(resolved) => {
                    bindings = resolved;
                    let import = |binding: &Binding| (binding.slot - self.slot(0)) / SLOT;
                    Decided::Stores(bindings.iter().map(|b| (import(b), b.function)).collect())
                }
                Imports::Unresolved(name) => Decided::Unresolved(name.to_str().unwrap().to_owned()),
            });
            Resolution {
                decided,
                bindings,
                asked_own_tree,
                asked_defined,
            }
        }
    }

    /// `Imports`, with each slot as the number of its import.
    #[derive(Debug, PartialEq, Eq)]
    enum Decided {
        Stores(Vec<(usize, usize)>),
        Unresolved(String),
    }

    struct Resolution {
        decided: Result<Decided, Unreadable>,
        bindings: Vec<Binding>,
        asked_own_tree: Vec<String>,
        asked_defined: Vec<String>,
    }

    impl Resolution {
        fn store(&self) {
            for binding in &self.bindings {
                // SAFETY: the slots are in the memory of the `Object` that the test still holds.
                unsafe { binding.store() };
            }
        }
    }

    /// What a slot holds until something is bound: in a real object, the address of its entry in the procedure linkage table.
    const LAZY: usize = 0x1a2b_3c00;
    const DEFLATE: usize = 0xdef1_a7e0;
    const INFLATE: usize = 0x1f1a_7e00;
    /// The host does not bind `inflateCopy`.
    const ZLIB: HostLibrary<'static> = HostLibrary {
        api: &[c"deflate", c"inflate", c"inflateCopy"],
        functions: &[(c"deflate", DEFLATE), (c"inflate", INFLATE)],
    };
    const NONE: [&str; 0] = [];

    /// A question to `is_defined`: the name, or the name and the version as `name@version`.
    fn question(name: &CStr, version: Option<&CStr>) -> String {
        let name = name.to_str().unwrap();
        match version {
            Some(version) => format!("{name}@{}", version.to_str().unwrap()),
            None => name.to_owned(),
        }
    }

    const IMPORTS: [Import; 3] = [import("first"), import("second"), import("third")];

    fn unresolved(name: &str) -> Result<Option<String>, Unreadable> {
        Ok(Some(name.to_owned()))
    }

    #[test]
    fn reads_tables_given_as_addresses() {
        let object = Object::new(Linked::AtZero, Pointers::Addresses, &IMPORTS);
        assert_eq!(
            object.walk(&["first"]),
            (
                unresolved("second"),
                vec!["first".to_owned(), "second".to_owned()]
            )
        );
        assert_eq!(object.walk(&["first", "second", "third"]).0, Ok(None));
    }

    #[test]
    fn reads_tables_given_as_offsets() {
        let object = Object::new(Linked::AtZero, Pointers::Offsets, &IMPORTS);
        assert_eq!(
            object.walk(&["first"]),
            (
                unresolved("second"),
                vec!["first".to_owned(), "second".to_owned()]
            )
        );
        assert_eq!(object.walk(&["first", "second", "third"]).0, Ok(None));
    }

    #[test]
    fn reads_tables_of_an_object_whose_base_is_above_them() {
        for pointers in [Pointers::Addresses, Pointers::Offsets] {
            let object = Object::new(Linked::AboveItsMapping, pointers, &IMPORTS);
            assert!(object.base() > object.memory.as_ptr() as usize + object.memory.len());
            assert_eq!(object.walk(&["first", "second"]).0, unresolved("third"));
        }
    }

    #[test]
    fn refuses_a_symbol_index_outside_the_range_of_the_symbol_table() {
        let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &IMPORTS);
        let entries_in_range = object.tables_size / SYMBOL;
        assert_eq!(entries_in_range * SYMBOL, object.tables_size);
        object.set_relocation(1, entries_in_range - 1, R_JUMP_SLOT);
        assert_eq!(
            object.walk(&["first"]),
            (
                unresolved("third"),
                vec!["first".to_owned(), "third".to_owned()]
            )
        );
        object.set_relocation(1, entries_in_range, R_JUMP_SLOT);
        assert_eq!(
            object.walk(&["first"]),
            (Err(Unreadable::SymbolIndex), vec!["first".to_owned()])
        );
    }

    #[test]
    fn refuses_a_symbol_in_another_segment() {
        let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &IMPORTS);
        assert_eq!(object.dynamic % SYMBOL, 0);
        object.set_relocation(0, object.dynamic / SYMBOL, R_JUMP_SLOT);
        assert_eq!(object.walk(&[]), (Err(Unreadable::SymbolIndex), vec![]));
    }

    #[test]
    fn refuses_a_symbol_name_that_does_not_end_inside_the_string_table() {
        let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &IMPORTS);
        object.set_symbol_name(0, object.strings_size - 1);
        assert_eq!(object.walk(&[""]).0, unresolved("second"));
        object.set_symbol_name(0, object.strings_size);
        assert_eq!(object.walk(&[""]), (Err(Unreadable::SymbolName), vec![]));

        let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &IMPORTS);
        object.set_dynamic(DT_STRSZ, object.strings_size as u64 - 1);
        assert_eq!(
            object.walk(&["first", "second"]),
            (
                Err(Unreadable::SymbolName),
                vec!["first".to_owned(), "second".to_owned()]
            )
        );
    }

    #[test]
    fn refuses_a_dynamic_section_without_an_end_inside_its_size() {
        let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &IMPORTS);
        object.dynamic_size = Some(object.dynamic_load_size - DYNAMIC_ENTRY / 2);
        assert_eq!(object.walk(&[]), (Err(Unreadable::Dynamic), vec![]));
    }

    #[test]
    fn refuses_a_dynamic_section_without_an_end_inside_its_segment() {
        let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &IMPORTS);
        object.dynamic_load_size -= DYNAMIC_ENTRY / 2;
        assert_eq!(object.walk(&[]), (Err(Unreadable::Dynamic), vec![]));
    }

    #[test]
    fn refuses_an_object_without_a_dynamic_section() {
        let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &IMPORTS);
        object.dynamic_size = None;
        assert_eq!(object.walk(&[]), (Err(Unreadable::Dynamic), vec![]));
    }

    #[test]
    fn refuses_relocations_that_do_not_end_inside_their_segment() {
        let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &IMPORTS);
        object.set_dynamic(DT_PLTRELSZ, (IMPORTS.len() * RELOCATION + 1) as u64);
        assert_eq!(
            object.walk(&["first", "second", "third"]),
            (Err(Unreadable::Relocations), vec![])
        );
    }

    #[test]
    fn refuses_relocations_of_another_format() {
        let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &IMPORTS);
        object.set_dynamic(DT_PLTREL, DT_REL);
        assert_eq!(object.walk(&[]), (Err(Unreadable::Relocations), vec![]));
    }

    #[test]
    fn refuses_a_table_outside_the_segments() {
        for pointers in [Pointers::Addresses, Pointers::Offsets] {
            for tag in [DT_JMPREL, DT_SYMTAB, DT_STRTAB] {
                let mut object = Object::new(Linked::AtZero, pointers, &IMPORTS);
                object.set_dynamic(tag, object.pointer(pointers, object.tables_size));
                assert_eq!(object.walk(&[]), (Err(Unreadable::Table), vec![]));
            }
        }
    }

    #[test]
    fn refuses_a_table_in_a_segment_that_is_not_readable() {
        let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &IMPORTS);
        object.tables_flags = 0;
        assert_eq!(object.walk(&[]), (Err(Unreadable::Table), vec![]));
    }

    #[test]
    fn refuses_relocations_without_a_symbol_table_or_a_string_table() {
        for tag in [DT_SYMTAB, DT_STRTAB] {
            let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &IMPORTS);
            object.set_dynamic_tag(tag, DT_DEBUG);
            assert_eq!(object.walk(&[]), (Err(Unreadable::Table), vec![]));
        }
    }

    #[test]
    fn an_object_without_lazy_relocations_has_no_unresolved_import() {
        let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &IMPORTS);
        object.set_dynamic_tag(DT_JMPREL, DT_DEBUG);
        assert_eq!(object.walk(&[]), (Ok(None), vec![]));
    }

    #[test]
    fn ignores_a_weak_import() {
        let weak = Import {
            binding: STB_WEAK,
            ..import("weak")
        };
        let object = Object::new(Linked::AtZero, Pointers::Addresses, &[weak]);
        assert_eq!(object.walk(&[]), (Ok(None), vec![]));
    }

    #[test]
    fn ignores_a_symbol_that_the_object_defines() {
        let own = Import {
            section: 1,
            ..import("own")
        };
        let object = Object::new(Linked::AtZero, Pointers::Addresses, &[own]);
        assert_eq!(object.walk(&[]), (Ok(None), vec![]));
    }

    #[test]
    fn ignores_a_relocation_that_is_not_a_jump_slot() {
        let data = Import {
            relocation_type: R_JUMP_SLOT ^ 1,
            ..import("data")
        };
        let object = Object::new(Linked::AtZero, Pointers::Addresses, &[data]);
        assert_eq!(object.walk(&[]), (Ok(None), vec![]));
    }

    fn asked(questions: &[&str]) -> Vec<String> {
        questions
            .iter()
            .map(|&question| question.to_owned())
            .collect()
    }

    /// The three ways in which a dynamic section says that the loader binds every import when it loads the object.
    const BOUND_AT_LOAD: [(i64, u64); 4] = [
        (DT_BIND_NOW, 0),
        (DT_FLAGS, DF_BIND_NOW),
        (DT_FLAGS, DF_BIND_NOW | 0x12),
        (DT_FLAGS_1, DF_1_NOW | 0x800_0000),
    ];

    #[test]
    fn an_object_that_is_bound_at_load_has_no_unresolved_import() {
        for (tag, value) in BOUND_AT_LOAD {
            for defers_got in [false, true] {
                let imports = [import("first"), got_import("second")];
                let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &imports);
                object.defers_got = defers_got;
                assert_eq!(object.walk(&[]).0, unresolved("first"));
                object.add_dynamic(tag, value);
                assert_eq!(object.walk(&[]), (Ok(None), vec![]), "{tag} {value}");
            }
        }
    }

    #[test]
    fn other_flags_do_not_say_that_an_object_is_bound_at_load() {
        for (tag, value) in [(DT_FLAGS, !DF_BIND_NOW), (DT_FLAGS_1, !DF_1_NOW)] {
            let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &IMPORTS);
            object.add_dynamic(tag, value);
            assert_eq!(
                object.walk(&[]),
                (unresolved("first"), asked(&["first"])),
                "{tag}"
            );
        }
    }

    #[test]
    fn reads_no_table_of_an_object_that_is_bound_at_load() {
        let changes: [fn(&mut Object); 5] = [
            |object| object.tables_flags = 0,
            |object| object.set_dynamic(DT_PLTREL, DT_REL),
            |object| object.set_dynamic(DT_PLTRELSZ, u64::MAX),
            |object| object.set_dynamic(DT_RELAENT, 1),
            |object| {
                object.set_dynamic(
                    DT_RELA,
                    object.pointer(Pointers::Addresses, object.tables_size),
                )
            },
        ];
        for (tag, value) in BOUND_AT_LOAD {
            for change in changes {
                let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &IMPORTS);
                object.defers_got = true;
                change(&mut object);
                assert!(object.walk(&[]).0.is_err());
                object.add_dynamic(tag, value);
                assert_eq!(object.walk(&[]), (Ok(None), vec![]), "{tag} {value}");
            }
        }
    }

    #[test]
    fn refuses_a_dynamic_section_without_an_end_that_says_bound_at_load() {
        for (tag, value) in BOUND_AT_LOAD {
            let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &IMPORTS);
            object.add_dynamic(tag, value);
            object.dynamic_size = Some(object.dynamic_load_size - DYNAMIC_ENTRY / 2);
            assert_eq!(object.walk(&[]), (Err(Unreadable::Dynamic), vec![]));
        }
    }

    /// `old` names the version of index 3, between two imports without a version.
    const VERSIONED_IMPORTS: [Import; 3] = [import("first"), versioned("old", 3), import("third")];

    fn versioned_object(pointers: Pointers) -> Object {
        let mut object = Object::new(Linked::AtZero, pointers, &VERSIONED_IMPORTS);
        object.set_version_needs(&[&[2, 3]]);
        object
    }

    #[test]
    fn asks_for_the_version_of_an_import_that_has_no_definition_by_its_name() {
        for pointers in [Pointers::Addresses, Pointers::Offsets] {
            let object = versioned_object(pointers);
            assert_eq!(
                object.walk(&["first", "old@VERSION_3", "third"]),
                (Ok(None), asked(&["first", "old", "old@VERSION_3", "third"]))
            );
            assert_eq!(
                object.walk(&["first", "old@VERSION_2", "third"]),
                (unresolved("old"), asked(&["first", "old", "old@VERSION_3"]))
            );
        }
    }

    #[test]
    fn reads_no_version_of_an_import_that_has_a_definition_by_its_name() {
        let object = versioned_object(Pointers::Addresses);
        assert_eq!(
            object.walk(&["first", "old", "old@VERSION_3", "third"]),
            (Ok(None), asked(&["first", "old", "third"]))
        );
    }

    #[test]
    fn asks_only_by_the_name_for_an_import_without_a_version() {
        // 0 and 1 are the indexes of a symbol without a version, also where an entry of the table claims them.
        for index in [0, 1, 0x8000, 0x8001] {
            let imports = [versioned("plain", index), import("other")];
            let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &imports);
            object.set_version_needs(&[&[0, 1, 2]]);
            assert_eq!(
                object.walk(&["plain@VERSION_2", "other"]),
                (unresolved("plain"), asked(&["plain"])),
                "{index}"
            );
        }
    }

    #[test]
    fn ignores_the_bit_of_a_hidden_version() {
        for (symbol, needed) in [(0x8003, 3), (3, 0x8003), (0x8003, 0x8003)] {
            let mut object = Object::new(
                Linked::AtZero,
                Pointers::Addresses,
                &[versioned("old", symbol)],
            );
            object.set_version_needs(&[&[2, needed]]);
            assert_eq!(
                object.walk(&["old@VERSION_3"]),
                (Ok(None), asked(&["old", "old@VERSION_3"]))
            );
        }
    }

    #[test]
    fn finds_a_version_in_each_entry_of_each_needed_library() {
        for index in 2..=5 {
            let mut object = Object::new(
                Linked::AboveItsMapping,
                Pointers::Offsets,
                &[versioned("old", index)],
            );
            object.set_version_needs(&[&[2, 3], &[4, 5]]);
            let version = format!("old@VERSION_{index}");
            assert_eq!(
                object.walk(&[&version]),
                (Ok(None), asked(&["old", &version]))
            );
        }
        // No entry has the index: the answer by the name stands.
        let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &[versioned("old", 6)]);
        object.set_version_needs(&[&[2, 3], &[4, 5]]);
        assert_eq!(object.walk(&[]), (unresolved("old"), asked(&["old"])));
    }

    #[test]
    fn version_tables_that_are_missing_or_malformed_name_no_version() {
        // The table is a need, its two entries, a second need and its two entries: 0 to 5. The last has the version of `old`.
        let outside = |object: &Object| object.pointer(Pointers::Addresses, object.tables_size);
        let cases: [(&str, &dyn Fn(&mut Object)); 18] = [
            ("no DT_VERSYM", &|object| {
                object.set_dynamic_tag(DT_VERSYM, DT_DEBUG)
            }),
            ("no DT_VERNEED", &|object| {
                object.set_dynamic_tag(DT_VERNEED, DT_DEBUG)
            }),
            ("no DT_VERNEEDNUM", &|object| {
                object.set_dynamic_tag(DT_VERNEEDNUM, DT_DEBUG)
            }),
            ("DT_VERNEEDNUM ends before the entry", &|object| {
                object.set_dynamic(DT_VERNEEDNUM, 1)
            }),
            ("vn_cnt ends before the entry", &|object| {
                object.write_u16(object.version_field(3, VN_CNT), 1)
            }),
            ("DT_VERSYM is outside the segments", &|object| {
                object.set_dynamic(DT_VERSYM, outside(object))
            }),
            ("DT_VERNEED is outside the segments", &|object| {
                object.set_dynamic(DT_VERNEED, outside(object))
            }),
            (
                "the element of DT_VERSYM is outside its segment",
                &|object| {
                    let end = object.versions + object.versions_size;
                    object.set_dynamic(DT_VERSYM, object.pointer(Pointers::Addresses, end - 2))
                },
            ),
            ("vn_next is its own entry", &|object| {
                object.write_u32(object.version_field(0, VN_NEXT), 0)
            }),
            ("vna_next is its own entry", &|object| {
                object.write_u32(object.version_field(4, VNA_NEXT), 0)
            }),
            (
                "vn_next is the first need again, if the offset wraps",
                &|object| {
                    let back = 0u32.wrapping_sub(3 * VERSION_ENTRY as u32);
                    object.write_u32(object.version_field(3, VN_NEXT), back);
                    object.write_u16(object.version_field(3, VN_CNT), 1);
                    object.set_dynamic(DT_VERNEEDNUM, u64::MAX)
                },
            ),
            (
                "vna_next is the entry before, if the offset wraps",
                &|object| {
                    let back = 0u32.wrapping_sub(VERSION_ENTRY as u32);
                    object.write_u32(object.version_field(5, VNA_NEXT), back);
                    object.write_u16(object.version_field(5, VNA_OTHER), 4);
                    object.write_u16(object.version_field(3, VN_CNT), u16::MAX)
                },
            ),
            ("vn_next is outside the segment", &|object| {
                object.write_u32(object.version_field(0, VN_NEXT), u32::MAX)
            }),
            ("vna_next is outside the segment", &|object| {
                object.write_u32(object.version_field(4, VNA_NEXT), u32::MAX)
            }),
            ("vn_aux is outside the segment", &|object| {
                let size = object.versions_size as u32;
                object.write_u32(object.version_field(3, VN_AUX), size)
            }),
            ("the entry ends outside the segment", &|object| {
                object.versions_size = object.version_field(6, 0) - 1 - object.versions
            }),
            ("the name is outside the string table", &|object| {
                let size = object.strings_size as u32;
                object.write_u32(object.version_field(5, VNA_NAME), size)
            }),
            ("the name does not end inside the string table", &|object| {
                // The string table ends with the name of the last import, which loses its end here.
                let last = object.strings_size - "last\0".len();
                object.write_u32(object.version_field(5, VNA_NAME), last as u32);
                object.set_dynamic(DT_STRSZ, object.strings_size as u64 - 1)
            }),
        ];
        for (case, change) in cases {
            let imports = [versioned("old", 5), import("last")];
            let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &imports);
            object.set_version_needs(&[&[2, 3], &[4, 5]]);
            assert_eq!(
                object.walk(&["old@VERSION_5", "last"]),
                (Ok(None), asked(&["old", "old@VERSION_5", "last"]))
            );
            change(&mut object);
            assert_eq!(
                object.walk(&["old@VERSION_5", "old@last", "last"]),
                (unresolved("old"), asked(&["old"])),
                "{case}"
            );
        }
    }

    #[test]
    fn counts_that_are_larger_than_the_chains_do_not_keep_the_walk_going() {
        let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &[versioned("old", 5)]);
        object.set_version_needs(&[&[2, 3], &[4, 5]]);
        object.set_dynamic(DT_VERNEEDNUM, u64::MAX);
        for need in [0, 3 * VERSION_ENTRY] {
            object.write_u16(object.version_needs + need + VN_CNT, u16::MAX);
        }
        assert_eq!(
            object.walk(&["old@VERSION_5"]),
            (Ok(None), asked(&["old", "old@VERSION_5"]))
        );
        // Without the wanted entry the walk reads both chains to their ends and stops there.
        object.write_u16(object.version_needs + 5 * VERSION_ENTRY + VNA_OTHER, 4);
        assert_eq!(object.walk(&[]), (unresolved("old"), asked(&["old"])));
    }

    #[test]
    fn a_chain_of_more_entries_than_an_object_can_name_versions_ends_the_walk() {
        // `links` entries without the version, and after them the need and the entry that have it. The links are needs without entries, or the entries of one need.
        let object = |links: usize, links_are_needs: bool| {
            let imports = [versioned("old", 2)];
            let room = (links + 3) * VERSION_ENTRY;
            let mut object = Object::with_version_needs_room(
                Linked::AtZero,
                Pointers::Addresses,
                &imports,
                room,
            );
            let chain = usize::from(!links_are_needs);
            if !links_are_needs {
                object.write_u16(object.version_field(0, VN_CNT), links as u16);
                object.write_u32(object.version_field(0, VN_AUX), VERSION_ENTRY as u32);
                let next = ((1 + links) * VERSION_ENTRY) as u32;
                object.write_u32(object.version_field(0, VN_NEXT), next);
            }
            for link in chain..chain + links {
                object.write_u32(object.version_field(link, VNA_NEXT), VERSION_ENTRY as u32);
            }
            if !links_are_needs {
                object.write_u32(object.version_field(links, VNA_NEXT), 0);
            }
            let need = chain + links;
            object.write_u16(object.version_field(need, VN_CNT), 1);
            object.write_u32(object.version_field(need, VN_AUX), VERSION_ENTRY as u32);
            object.write_u16(object.version_field(need + 1, VNA_OTHER), 2);
            object.write_u32(object.version_field(need + 1, VNA_NAME), version_name(2));
            object.set_dynamic(DT_VERNEEDNUM, u64::MAX);
            object
        };
        for links_are_needs in [true, false] {
            assert_eq!(
                object(1000, links_are_needs).walk(&["old@VERSION_2"]),
                (Ok(None), asked(&["old", "old@VERSION_2"]))
            );
            assert_eq!(
                object(usize::from(u16::MAX), links_are_needs).walk(&["old@VERSION_2"]),
                (unresolved("old"), asked(&["old"]))
            );
        }
    }

    /// A jump slot, an import through the global offset table, and a jump slot that the linker put in the table of `DT_RELA`.
    const MUSL_IMPORTS: [Import; 3] = [
        import("first"),
        got_import("second"),
        Import {
            relocation_type: R_JUMP_SLOT,
            ..got_import("third")
        },
    ];

    #[test]
    fn reads_the_table_of_dt_rela_only_for_a_loader_that_defers_its_imports() {
        for pointers in [Pointers::Addresses, Pointers::Offsets] {
            let mut object = Object::new(Linked::AboveItsMapping, pointers, &MUSL_IMPORTS);
            assert_eq!(object.walk(&["first"]), (Ok(None), asked(&["first"])));

            object.defers_got = true;
            assert_eq!(
                object.walk(&["first"]),
                (unresolved("second"), asked(&["first", "second"]))
            );
            assert_eq!(
                object.walk(&["first", "second"]),
                (unresolved("third"), asked(&["first", "second", "third"]))
            );
            assert_eq!(object.walk(&["first", "second", "third"]).0, Ok(None));
        }
    }

    #[test]
    fn reports_a_got_import_in_the_table_of_dt_jmprel_only_for_a_loader_that_defers_it() {
        let data = Import {
            relocation_type: R_GLOB_DAT,
            ..import("data")
        };
        let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &[data]);
        assert_eq!(object.walk(&[]), (Ok(None), vec![]));
        object.defers_got = true;
        assert_eq!(object.walk(&[]), (unresolved("data"), asked(&["data"])));
    }

    #[test]
    fn reads_the_table_of_dt_jmprel_before_that_of_dt_rela() {
        let imports = [got_import("data"), import("function")];
        let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &imports);
        object.defers_got = true;
        assert_eq!(
            object.walk(&[]),
            (unresolved("function"), asked(&["function"]))
        );
        assert_eq!(
            object.walk(&["function"]),
            (unresolved("data"), asked(&["function", "data"]))
        );
    }

    #[test]
    fn reads_the_table_of_dt_rela_of_an_object_without_lazy_relocations() {
        let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &MUSL_IMPORTS);
        object.defers_got = true;
        object.set_dynamic_tag(DT_JMPREL, DT_DEBUG);
        assert_eq!(object.walk(&[]), (unresolved("second"), asked(&["second"])));
        object.set_dynamic_tag(DT_RELA, DT_DEBUG);
        assert_eq!(object.walk(&[]), (Ok(None), vec![]));
    }

    #[test]
    fn skips_other_relocations_for_a_loader_that_defers_got_imports() {
        // Neither a jump slot nor a `GLOB_DAT`: among them the copy and the relative relocation of each processor.
        for relocation_type in [0, 1, R_GLOB_DAT - 1, R_JUMP_SLOT + 1] {
            for relocated_in in [Section::RelaPlt, Section::RelaDyn] {
                let other = Import {
                    relocation_type,
                    relocated_in,
                    ..import("other")
                };
                let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &[other]);
                object.defers_got = true;
                assert_eq!(object.walk(&[]), (Ok(None), vec![]), "{relocation_type}");
            }
        }
    }

    #[test]
    fn ignores_a_got_import_that_is_weak_or_that_the_object_defines() {
        let weak = Import {
            binding: STB_WEAK,
            ..got_import("weak")
        };
        let own = Import {
            section: 1,
            ..got_import("own")
        };
        let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &[weak, own]);
        object.defers_got = true;
        assert_eq!(object.walk(&[]), (Ok(None), vec![]));
    }

    #[test]
    fn asks_for_the_version_of_a_got_import() {
        let imports = [Import {
            version: 4,
            ..got_import("old")
        }];
        let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &imports);
        object.set_version_needs(&[&[4]]);
        object.defers_got = true;
        assert_eq!(
            object.walk(&["old@VERSION_4"]),
            (Ok(None), asked(&["old", "old@VERSION_4"]))
        );
    }

    #[test]
    fn refuses_another_entry_size_only_where_it_reads_the_table_of_dt_rela() {
        for size in [0, 16, 25] {
            let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &MUSL_IMPORTS);
            object.set_dynamic(DT_RELAENT, size);
            assert_eq!(object.walk(&["first"]), (Ok(None), asked(&["first"])));
            object.defers_got = true;
            assert_eq!(
                object.walk(&["first"]),
                (Err(Unreadable::Relocations), vec![])
            );
        }
        // The loaders do not read the entry, so an object without one is read as well.
        let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &MUSL_IMPORTS);
        object.defers_got = true;
        object.set_dynamic_tag(DT_RELAENT, DT_DEBUG);
        assert_eq!(object.walk(&["first"]).0, unresolved("second"));
    }

    #[test]
    fn refuses_a_table_of_dt_rela_outside_the_segments_only_where_it_reads_it() {
        let changes: [(Unreadable, fn(&mut Object)); 2] = [
            (Unreadable::Table, |object| {
                object.set_dynamic(
                    DT_RELA,
                    object.pointer(Pointers::Addresses, object.tables_size),
                )
            }),
            (Unreadable::Relocations, |object| {
                object.set_dynamic(DT_RELASZ, (object.versions_size + 1) as u64)
            }),
        ];
        for (error, change) in changes {
            let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &MUSL_IMPORTS);
            change(&mut object);
            assert_eq!(object.walk(&["first"]), (Ok(None), asked(&["first"])));
            object.defers_got = true;
            assert_eq!(object.walk(&["first"]), (Err(error), vec![]));
        }
    }

    const ZLIB_IMPORTS: [Import; 3] = [import("other"), import("deflate"), import("inflate")];
    const UNBOUND: [usize; 3] = [LAZY, LAZY + 1, LAZY + 2];

    fn stores(bindings: &[(usize, usize)]) -> Result<Decided, Unreadable> {
        Ok(Decided::Stores(bindings.to_vec()))
    }

    fn refused(name: &str) -> Result<Decided, Unreadable> {
        Ok(Decided::Unresolved(name.to_owned()))
    }

    #[test]
    fn checks_every_import_of_an_object_that_imports_nothing_of_the_host_library() {
        let object = Object::new(Linked::AtZero, Pointers::Addresses, &IMPORTS);
        let resolution = object.resolve(&["second"], &["first"]);
        assert_eq!(resolution.decided, refused("second"));
        assert_eq!(resolution.asked_defined, ["first", "second"]);
        assert_eq!(resolution.asked_own_tree, NONE);

        let resolution = object.resolve(&[], &["first", "second", "third"]);
        assert_eq!(resolution.decided, stores(&[]));
        assert_eq!(resolution.asked_own_tree, NONE);
    }

    #[test]
    fn binds_nothing_when_the_own_tree_defines_every_import_of_the_library() {
        let imports = [import("deflate"), import("other"), import("inflateCopy")];
        let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &imports);
        let own = ["deflate", "inflateCopy"];
        let resolution = object.resolve(&own, &["other"]);
        assert_eq!(resolution.decided, stores(&[]));
        assert_eq!(resolution.asked_own_tree, own);
        assert_eq!(resolution.asked_defined, ["other"]);

        // Nothing is stored, so the slots may be read-only.
        object.slots_flags = PF_R;
        assert_eq!(object.resolve(&own, &["other"]).decided, stores(&[]));
    }

    #[test]
    fn refuses_an_object_whose_own_tree_defines_a_part_of_the_library() {
        let object = Object::new(Linked::AtZero, Pointers::Addresses, &ZLIB_IMPORTS);
        for (own, missing) in [("inflate", "deflate"), ("deflate", "inflate")] {
            let resolution = object.resolve(&[own], &["other"]);
            assert_eq!(resolution.decided, refused(missing));
            assert_eq!(resolution.asked_own_tree, ["deflate", "inflate"]);
        }
        assert_eq!(object.slot_values(), UNBOUND);
    }

    #[test]
    fn binds_the_host_library_when_the_own_tree_defines_none_of_it() {
        for linked in [Linked::AtZero, Linked::AboveItsMapping] {
            let object = Object::new(linked, Pointers::Offsets, &ZLIB_IMPORTS);
            // Another library of the process that defines the names does not change which implementation the object gets.
            let resolution = object.resolve(&[], &["other", "deflate", "inflate"]);
            assert_eq!(resolution.decided, stores(&[(1, DEFLATE), (2, INFLATE)]));
            assert_eq!(resolution.asked_own_tree, ["deflate", "inflate"]);
            assert_eq!(resolution.asked_defined, ["other"]);
            assert_eq!(object.slot_values(), UNBOUND);

            resolution.store();
            assert_eq!(object.slot_values(), [LAZY, DEFLATE, INFLATE]);
            resolution.store();
            assert_eq!(object.slot_values(), [LAZY, DEFLATE, INFLATE]);
        }
    }

    #[test]
    fn refuses_an_import_of_the_library_that_the_host_does_not_bind() {
        let imports = [import("deflate"), import("inflateCopy"), import("inflate")];
        let object = Object::new(Linked::AtZero, Pointers::Addresses, &imports);
        // A definition in another library of the process is of another implementation.
        let resolution = object.resolve(&[], &["deflate", "inflateCopy", "inflate"]);
        assert_eq!(resolution.decided, refused("inflateCopy"));
        assert_eq!(
            resolution.asked_own_tree,
            ["deflate", "inflateCopy", "inflate"]
        );
        assert_eq!(resolution.asked_defined, NONE);
        assert_eq!(object.slot_values(), UNBOUND);

        let resolution = object.resolve(&["deflate"], &[]);
        assert_eq!(resolution.decided, refused("inflateCopy"));
    }

    #[test]
    fn refuses_an_import_that_nothing_defines_beside_imports_that_the_host_binds() {
        let imports = [import("deflate"), import("missing"), import("inflate")];
        let object = Object::new(Linked::AtZero, Pointers::Addresses, &imports);
        let resolution = object.resolve(&[], &[]);
        assert_eq!(resolution.decided, refused("missing"));
        assert_eq!(resolution.asked_own_tree, ["deflate"]);
        assert_eq!(resolution.asked_defined, ["missing"]);
        assert_eq!(object.slot_values(), UNBOUND);

        // The walk ends at an import outside the library that nothing defines, so that one is named.
        let imports = [import("inflateCopy"), import("missing")];
        let object = Object::new(Linked::AtZero, Pointers::Addresses, &imports);
        assert_eq!(object.resolve(&[], &[]).decided, refused("missing"));
    }

    #[test]
    fn stores_nothing_in_an_object_whose_tables_it_cannot_read_to_the_end() {
        let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &ZLIB_IMPORTS);
        object.set_relocation(2, object.tables_size / SYMBOL, R_JUMP_SLOT);
        let resolution = object.resolve(&[], &["other"]);
        assert_eq!(resolution.decided, Err(Unreadable::SymbolIndex));
        assert_eq!(resolution.asked_own_tree, ["deflate"]);
        assert_eq!(object.slot_values(), UNBOUND);
    }

    #[test]
    fn refuses_a_slot_that_is_not_a_writable_pointer() {
        let object = || Object::new(Linked::AtZero, Pointers::Addresses, &ZLIB_IMPORTS);
        let slots = object().slots;
        let cases: [(&str, fn(&mut Object)); 5] = [
            ("in a segment that is not writable", |object| {
                object.slots_flags = PF_R
            }),
            ("outside the segments", |object| {
                object.set_relocation_offset(2, object.slots + object.slots_size)
            }),
            ("without room for a pointer", |object| {
                object.slots_size -= SLOT / 2
            }),
            ("not aligned", |object| {
                object.set_relocation_offset(1, object.slots + SLOT + SLOT / 2)
            }),
            ("read-only after relocation", |object| {
                object.read_only_after_relocation = Some((object.slots + SLOT, SLOT))
            }),
        ];
        for (case, change) in cases {
            let mut object = object();
            change(&mut object);
            let resolution = object.resolve(&[], &["other"]);
            assert_eq!(resolution.decided, Err(Unreadable::Slot), "{case}");
            assert_eq!(object.slot_values(), UNBOUND, "{case}");
        }

        // Only the slots that get a function have to be writable.
        let mut object = object();
        object.read_only_after_relocation = Some((slots, SLOT));
        object.set_relocation_offset(0, slots + object.slots_size);
        let resolution = object.resolve(&[], &["other"]);
        assert_eq!(resolution.decided, stores(&[(1, DEFLATE), (2, INFLATE)]));
    }

    #[test]
    fn does_not_write_to_a_slot_that_holds_the_function() {
        let protection = libc::PROT_READ | libc::PROT_WRITE;
        let flags = libc::MAP_PRIVATE | libc::MAP_ANONYMOUS;
        // SAFETY: maps a new page.
        let page = unsafe { libc::mmap(core::ptr::null_mut(), SLOT, protection, flags, -1, 0) };
        assert_ne!(page, libc::MAP_FAILED);
        let binding = Binding {
            slot: page.addr(),
            function: DEFLATE,
        };
        // SAFETY: the slot is the start of the page, which is writable.
        unsafe { binding.store() };
        // SAFETY: `page` is the mapping above.
        assert_eq!(unsafe { libc::mprotect(page, SLOT, libc::PROT_READ) }, 0);
        // SAFETY: the page is readable, and a write to it would end the test with SIGSEGV.
        unsafe { binding.store() };
        // SAFETY: the page is readable.
        assert_eq!(unsafe { page.cast::<usize>().read() }, DEFLATE);
        // SAFETY: `page` is the mapping above.
        assert_eq!(unsafe { libc::munmap(page, SLOT) }, 0);
    }

    #[test]
    fn binds_nothing_in_an_object_that_is_bound_at_load() {
        for (tag, value) in BOUND_AT_LOAD {
            let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &ZLIB_IMPORTS);
            object.add_dynamic(tag, value);
            // The loader bound the imports of the library too, so the object has all of them from one implementation.
            let resolution = object.resolve(&[], &[]);
            assert_eq!(resolution.decided, stores(&[]), "{tag} {value}");
            assert_eq!(resolution.asked_own_tree, NONE);
            assert_eq!(resolution.asked_defined, NONE);
            resolution.store();
            assert_eq!(object.slot_values(), UNBOUND);
        }
    }

    #[test]
    fn asks_for_the_version_of_an_import_outside_the_library() {
        let imports = [versioned("other", 3), import("deflate")];
        let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &imports);
        object.set_version_needs(&[&[2, 3]]);
        let resolution = object.resolve(&[], &["other@VERSION_3"]);
        assert_eq!(resolution.decided, stores(&[(1, DEFLATE)]));
        assert_eq!(resolution.asked_defined, ["other", "other@VERSION_3"]);

        let resolution = object.resolve(&[], &["other@VERSION_2"]);
        assert_eq!(resolution.decided, refused("other"));
        assert_eq!(resolution.asked_defined, ["other", "other@VERSION_3"]);

        let resolution = object.resolve(&[], &["other", "other@VERSION_3"]);
        assert_eq!(resolution.decided, stores(&[(1, DEFLATE)]));
        assert_eq!(resolution.asked_defined, ["other"]);
    }

    #[test]
    fn decides_an_import_of_the_library_by_its_name_whatever_version_it_names() {
        let imports = [
            versioned("deflate", 3),
            versioned("inflate", 3),
            import("other"),
        ];
        let mut object = Object::new(Linked::AtZero, Pointers::Addresses, &imports);
        object.set_version_needs(&[&[2, 3]]);
        // The own tree has none by the name: the host's are bound, whatever the process defines by the name or by the version.
        let defined = ["other", "deflate", "deflate@VERSION_3", "inflate"];
        let resolution = object.resolve(&[], &defined);
        assert_eq!(resolution.decided, stores(&[(0, DEFLATE), (1, INFLATE)]));
        assert_eq!(resolution.asked_own_tree, ["deflate", "inflate"]);
        assert_eq!(resolution.asked_defined, ["other"]);

        // The own tree has them by the name: nothing is bound, and nothing asks for the version.
        let resolution = object.resolve(&["deflate", "inflate"], &["other"]);
        assert_eq!(resolution.decided, stores(&[]));
        assert_eq!(resolution.asked_own_tree, ["deflate", "inflate"]);
        assert_eq!(resolution.asked_defined, ["other"]);

        let resolution = object.resolve(&["deflate"], &["other", "inflate@VERSION_3"]);
        assert_eq!(resolution.decided, refused("inflate"));
    }

    #[test]
    fn leaves_the_imports_through_the_global_offset_table_to_the_loader() {
        // glibc binds these in dlopen(), which fails if nothing defines one: the decision never reads the table of `DT_RELA`.
        let imports = [
            got_import("deflate"),
            got_import("missing"),
            import("inflate"),
        ];
        let object = Object::new(Linked::AtZero, Pointers::Addresses, &imports);
        let resolution = object.resolve(&[], &[]);
        assert_eq!(resolution.decided, stores(&[(2, INFLATE)]));
        assert_eq!(resolution.asked_own_tree, ["inflate"]);
        assert_eq!(resolution.asked_defined, NONE);
    }
}
