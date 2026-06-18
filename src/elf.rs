use goblin::elf::{
    Elf,
    program_header::{PT_LOAD, ProgramHeader},
};
use libafl::prelude::*;

pub(crate) fn elf_vma2offset(
    elf_bytes: &[u8],
    vma_start: u64,
    vma_end: u64,
) -> Result<usize, Error> {
  if vma_start > vma_end {
    return Err(Error::illegal_argument(format!("vma_start({vma_start:#x}) > vma_end({vma_end:#x})")));
  }
    let elf = Elf::parse(elf_bytes)
        .map_err(|err| Error::illegal_argument(format!("Failed to parse ELF input: {err}")))?;
    elf_vma2offset_by_ph(
        elf_bytes.len(),
        elf.program_headers
            .iter()
            .filter(|header| header.p_type == PT_LOAD),
        vma_start,
        vma_end,
    )
}

fn elf_vma2offset_by_ph<'a>(
    elf_size: usize,
    segments: impl IntoIterator<Item = &'a ProgramHeader>,
    vma_start: u64,
    vma_end: u64,
) -> Result<usize, Error> {
    for segment in segments {
        let Some(file_vaddr_end) = segment.p_vaddr.checked_add(segment.p_filesz) else {
            continue;
        };
        let Some(mem_vaddr_end) = segment.p_vaddr.checked_add(segment.p_memsz) else {
            continue;
        };

        if vma_start < segment.p_vaddr || vma_end > file_vaddr_end {
            if vma_start >= segment.p_vaddr && vma_start < mem_vaddr_end {
                return Err(Error::illegal_argument(format!(
                    "VMA range {:#x}..{:#x} is mapped by ELF segment {:#x}..{:#x}, but not backed by file bytes",
                    vma_start, vma_end, segment.p_vaddr, mem_vaddr_end
                )));
            }
            continue;
        }

        let Some(segment_offset) = vma_start.checked_sub(segment.p_vaddr) else {
            continue;
        };
        let Some(file_offset) = segment.p_offset.checked_add(segment_offset) else {
            continue;
        };
        let Some(file_end) = file_offset.checked_add(vma_end - vma_start) else {
            continue;
        };

        let elf_size = u64::try_from(elf_size).map_err(|_| {
            Error::illegal_argument(format!("ELF input size {elf_size} does not fit in u64"))
        })?;

        if file_end > elf_size {
            return Err(Error::illegal_argument(format!(
                "ELF segment maps VMA {vma_start:#x} to file range {file_offset:#x}..{file_end:#x}, beyond input length {elf_size:#x}"
            )));
        }

        return usize::try_from(file_offset).map_err(|_| {
            Error::illegal_argument(format!(
                "ELF file offset {file_offset:#x} does not fit in usize"
            ))
        });
    }

    Err(Error::illegal_argument(format!(
        "VMA range {vma_start:#x}..{vma_end:#x} is not covered by a loadable ELF file segment"
    )))
}
