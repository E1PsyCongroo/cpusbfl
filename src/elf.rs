use goblin::elf::{
    Elf,
    header::{EI_CLASS, EI_DATA, ELFCLASS32, ELFCLASS64, ELFDATA2LSB, ELFDATA2MSB},
    program_header::{PT_LOAD, ProgramHeader},
};

#[derive(Clone, Debug)]
pub(crate) struct ExecutableSection {
    pub(crate) name: String,
    pub(crate) vma_start: u64,
    pub(crate) vma_end: u64,
    pub(crate) file_offset: usize,
    pub(crate) file_end: usize,
    pub(crate) sh_size_offset: usize,
    pub(crate) sh_size_width: usize,
    pub(crate) little_endian: bool,
}

impl ExecutableSection {
    pub(crate) fn size(&self) -> u64 {
        self.vma_end - self.vma_start
    }

    pub(crate) fn contains_range(&self, vma_start: u64, vma_end: u64) -> bool {
        self.vma_start <= vma_start && vma_end <= self.vma_end
    }

    pub(crate) fn vma_to_offset(&self, vma: u64) -> Option<usize> {
        if !self.contains_range(vma, vma) {
            return None;
        }

        let offset = usize::try_from(vma.checked_sub(self.vma_start)?).ok()?;
        self.file_offset.checked_add(offset)
    }
}

pub(crate) fn elf_vma_to_offset(
    elf_bytes: &[u8],
    vma_start: u64,
    vma_end: u64,
) -> Result<usize, Box<dyn std::error::Error>> {
    if vma_start > vma_end {
        return Err((format!("vma_start({vma_start:#x}) > vma_end({vma_end:#x})")).into());
    }
    let elf = Elf::parse(elf_bytes).map_err(|err| format!("Failed to parse ELF input: {err}"))?;
    elf_vma_to_offset_by_ph(
        elf_bytes.len(),
        elf.program_headers
            .iter()
            .filter(|header| header.p_type == PT_LOAD),
        vma_start,
        vma_end,
    )
}

fn elf_vma_to_offset_by_ph<'a>(
    elf_size: usize,
    segments: impl IntoIterator<Item = &'a ProgramHeader>,
    vma_start: u64,
    vma_end: u64,
) -> Result<usize, Box<dyn std::error::Error>> {
    for segment in segments {
        let Some(file_vaddr_end) = segment.p_vaddr.checked_add(segment.p_filesz) else {
            continue;
        };
        let Some(mem_vaddr_end) = segment.p_vaddr.checked_add(segment.p_memsz) else {
            continue;
        };

        if vma_start < segment.p_vaddr || vma_end > file_vaddr_end {
            if vma_start >= segment.p_vaddr && vma_start < mem_vaddr_end {
                return Err(format!(
                    "VMA range {:#x}..{:#x} is mapped by ELF segment {:#x}..{:#x}, but not backed by file bytes",
                    vma_start, vma_end, segment.p_vaddr, mem_vaddr_end
                ).into());
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

        let elf_size = u64::try_from(elf_size)
            .map_err(|_| format!("ELF input size {elf_size} does not fit in u64"))?;

        if file_end > elf_size {
            return Err(format!(
                "ELF segment maps VMA {vma_start:#x} to file range {file_offset:#x}..{file_end:#x}, beyond input length {elf_size:#x}"
            ).into());
        }

        return usize::try_from(file_offset)
            .map_err(|_| format!("ELF file offset {file_offset:#x} does not fit in usize").into());
    }

    Err(format!(
        "VMA range {vma_start:#x}..{vma_end:#x} is not covered by a loadable ELF file segment"
    )
    .into())
}

pub(crate) fn parse_executable_sections(
    elf_bytes: &[u8],
) -> Result<Vec<ExecutableSection>, Box<dyn std::error::Error>> {
    let elf = Elf::parse(elf_bytes).map_err(|err| format!("Failed to parse ELF input: {err}"))?;
    let shoff = usize::try_from(elf.header.e_shoff).map_err(|_| {
        format!(
            "ELF section header offset {:#x} does not fit in usize",
            elf.header.e_shoff
        )
    })?;
    let shentsize = usize::from(elf.header.e_shentsize);
    let little_endian = match elf.header.e_ident[EI_DATA] {
        ELFDATA2LSB => true,
        ELFDATA2MSB => false,
        data => {
            return Err(format!("Unsupported ELF endianness: {data}").into());
        }
    };
    let (sh_size_field_offset, sh_size_width) = match elf.header.e_ident[EI_CLASS] {
        ELFCLASS32 => (20usize, 4usize),
        ELFCLASS64 => (32usize, 8usize),
        class => return Err(format!("Unsupported ELF class: {class}").into()),
    };

    let mut sections = Vec::new();
    for (idx, section) in elf.section_headers.iter().enumerate() {
        if !section.is_executable() || section.sh_size == 0 {
            continue;
        }

        let Some(file_range) = section.file_range() else {
            continue;
        };
        let Some(section_name) = elf.shdr_strtab.get_at(section.sh_name) else {
            return Err(format!(
        "Failed to resolve executable section name: sh_name={} is not a valid offset in .shstrtab",
        section.sh_name
    )
    .into());
        };
        if file_range.end > elf_bytes.len() {
            return Err(format!(
                "Executable section '{}' file range {:#x}..{:#x} exceeds ELF length {:#x}",
                section_name,
                file_range.start,
                file_range.end,
                elf_bytes.len()
            )
            .into());
        }

        let Some(vma_end) = section.sh_addr.checked_add(section.sh_size) else {
            return Err(format!(
                "Executable section '{}' VMA range overflows at {:#x} + {:#x}",
                section_name, section.sh_addr, section.sh_size
            )
            .into());
        };
        let Some(sh_entry_offset) = shoff.checked_add(idx.saturating_mul(shentsize)) else {
            return Err(format!(
                "Executable section '{section_name}' section header offset overflows"
            )
            .into());
        };
        let Some(sh_size_offset) = sh_entry_offset.checked_add(sh_size_field_offset) else {
            return Err(format!(
                "Executable section '{section_name}' sh_size field offset overflows"
            )
            .into());
        };
        let Some(sh_size_end) = sh_size_offset.checked_add(sh_size_width) else {
            return Err(
                format!("Executable section '{section_name}' sh_size field end overflows").into(),
            );
        };
        if sh_size_end > elf_bytes.len() {
            return Err(format!(
                "Executable section '{section_name}' sh_size field is outside ELF bytes"
            )
            .into());
        }

        sections.push(ExecutableSection {
            name: section_name.to_string(),
            vma_start: section.sh_addr,
            vma_end,
            file_offset: file_range.start,
            file_end: file_range.end,
            sh_size_offset,
            sh_size_width,
            little_endian,
        });
    }

    sections.sort_by_key(|section| section.vma_start);
    Ok(sections)
}

pub(crate) fn executable_section_containing_vma<'a>(
    sections: &'a [ExecutableSection],
    vma_start: u64,
    vma_end: u64
) -> Option<&'a ExecutableSection> {
    sections
        .iter()
        .find(|section| section.contains_range(vma_start, vma_end))
}

pub(crate) fn write_executable_section_size(
    elf_bytes: &mut [u8],
    section: &ExecutableSection,
    new_size: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let old_size = section.size();

    if new_size > old_size {
        return Err(format!(
            "New section size {new_size:#x} is larger than original section size {old_size:#x}; \
             Executable section: '{}', VMA {:#x}..{:#x}, file range {:#x}..{:#x}",
            section.name, section.vma_start, section.vma_end, section.file_offset, section.file_end,
        )
        .into());
    }

    let end = section
        .sh_size_offset
        .checked_add(section.sh_size_width)
        .ok_or_else(|| {
            format!(
                "Executable section: '{}' sh_size field end offset overflow: offset={:#x}, width={}",
                section.name, section.sh_size_offset, section.sh_size_width
            )
        })?;

    let elf_len = elf_bytes.len();
    let field = elf_bytes
        .get_mut(section.sh_size_offset..end)
        .ok_or_else(|| {
            format!(
                "Executable section: '{}' sh_size field range {:#x}..{:#x} exceeds ELF length {:#x}",
                section.name, section.sh_size_offset, end, elf_len,
            )
        })?;

    match section.sh_size_width {
        4 => {
            let new_size = u32::try_from(new_size).map_err(|_| {
                format!(
                    "Executable section: '{}' new section size {:#x} does not fit in ELF32 sh_size",
                    section.name, new_size
                )
            })?;

            if section.little_endian {
                field.copy_from_slice(&new_size.to_le_bytes());
            } else {
                field.copy_from_slice(&new_size.to_be_bytes());
            }
        }
        8 => {
            if section.little_endian {
                field.copy_from_slice(&new_size.to_le_bytes());
            } else {
                field.copy_from_slice(&new_size.to_be_bytes());
            }
        }
        width => {
            return Err(format!("Unsupported sh_size field width: {width}").into());
        }
    }

    Ok(())
}
