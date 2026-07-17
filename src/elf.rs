use lief::generic::{Binary as _, Section};
use ouroboros::self_referencing;
use tempfile::NamedTempFile;

pub(crate) fn section_contains_range(
    section: &lief::elf::Section,
    vma_start: u64,
    vma_end: u64,
) -> bool {
    let Some(section_end) = section.virtual_address().checked_add(section.size()) else {
        return false;
    };

    section.virtual_address() <= vma_start && vma_end <= section_end
}

pub(crate) fn section_vma2offset(
    section: &lief::elf::Section,
    vma_start: u64,
    vma_end: u64,
) -> Option<u64> {
    section_contains_range(section, vma_start, vma_end)
        .then(|| vma_start.checked_sub(section.virtual_address()))?
        .and_then(|offset| section.file_offset().checked_add(offset))
}

fn align_up(value: u64, align: u64) -> Result<u64, Box<dyn std::error::Error>> {
    if align <= 1 {
        return Ok(value);
    }

    let rem = value % align;
    if rem == 0 {
        return Ok(value);
    }

    value
        .checked_add(align - rem)
        .ok_or_else(|| format!("align_up overflow: value={value:#x}, align={align:#x}").into())
}

fn segment_vaddr_end(segment: &lief::elf::Segment) -> Result<u64, Box<dyn std::error::Error>> {
    segment
        .virtual_address()
        .checked_add(segment.virtual_size())
        .ok_or_else(|| {
            format!(
                "Segment VMA overflow: start={:#x}, mem_size={:#x}",
                segment.virtual_address(),
                segment.virtual_size(),
            )
            .into()
        })
}

#[self_referencing]
pub(crate) struct ELFParser {
    pub elf: lief::elf::Binary,

    #[borrows(elf)]
    #[covariant]
    pub executable_sections: Vec<lief::elf::Section<'this>>,

    #[borrows(elf)]
    #[covariant]
    pub text_section: Option<lief::elf::Section<'this>>,

    #[borrows(elf)]
    #[covariant]
    pub load_segments: Vec<lief::elf::Segment<'this>>,
}

impl From<lief::elf::Binary> for ELFParser {
    fn from(elf: lief::elf::Binary) -> Self {
        ELFParserBuilder {
            elf,
            executable_sections_builder: |elf| {
                elf.sections()
                    .filter(|section| {
                        section
                            .flags()
                            .contains(lief::elf::section::Flags::EXECINSTR)
                            && section.get_type() == lief::elf::section::Type::PROGBITS
                    })
                    .collect()
            },
            text_section_builder: |elf| {
                elf.section_by_name(".text").filter(|section| {
                    section
                        .flags()
                        .contains(lief::elf::section::Flags::EXECINSTR)
                        && section.get_type() == lief::elf::section::Type::PROGBITS
                })
            },
            load_segments_builder: |elf| {
                elf.segments()
                    .filter(|s| s.p_type() == lief::elf::segment::Type::LOAD)
                    .collect()
            },
        }
        .build()
    }
}

impl TryFrom<ELFParser> for Vec<u8> {
    type Error = Box<dyn std::error::Error>;

    fn try_from(parser: ELFParser) -> Result<Self, Self::Error> {
        let tmp = NamedTempFile::new()?;
        let path = tmp.path().to_path_buf();

        parser.into_heads().elf.write(&path);

        let bytes = std::fs::read(&path)?;
        Ok(bytes)
    }
}

impl ELFParser {
    pub fn from_bytes(elf_bytes: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
        let Some(lief::Binary::ELF(elf)) = lief::Binary::from(&mut std::io::Cursor::new(elf_bytes))
        else {
            return Err("Failed to parse ELF input".into());
        };

        Ok(ELFParser::from(elf))
    }

    pub fn vma2offset(&self, vma: u64) -> Result<u64, Box<dyn std::error::Error>> {
        self.borrow_elf()
            .virtual_address_to_offset(vma)
            .map_err(|e| e.to_string().into())
    }

    pub fn prepare_for_code_segment_insert(self) -> Result<Self, Box<dyn std::error::Error>> {
        let mut elf = self.into_heads().elf;

        if elf.header().file_type() != lief::elf::header::FileType::EXEC {
            return Err("code segment insertion currently only supports ET_EXEC ELF files".into());
        }

        let phdr_offset = elf.relocate_phdr_table(lief::elf::binary::PhdrReloc::AUTO);
        if phdr_offset == 0 {
            return Err("failed to relocate the ELF program header table".into());
        }

        Ok(Self::from(elf))
    }

    pub fn code_segment_mapped_size(
        &self,
        code_size: u64,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        if code_size == 0 {
            return Err("code segment must not be empty".into());
        }

        let page_size = self.borrow_elf().page_size();
        if page_size == 0 {
            return Err("LIEF reported a zero ELF page size".into());
        }

        align_up(code_size, page_size)
    }

    pub fn patch_vaddr(self, vaddr: u64, patch: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
        if patch.is_empty() {
            return Ok(self);
        }

        let patch_size = u64::try_from(patch.len())?;
        let patch_end = vaddr.checked_add(patch_size).ok_or_else(|| {
            format!("patch range overflow: start={vaddr:#x}, size={patch_size:#x}")
        })?;

        let range_is_file_backed = self.borrow_load_segments().iter().any(|segment| {
            let segment_start = segment.virtual_address();
            let Some(segment_end) = segment_start.checked_add(segment.physical_size()) else {
                return false;
            };
            segment_start <= vaddr && patch_end <= segment_end
        });
        if !range_is_file_backed {
            return Err(format!(
                "patch range {vaddr:#x}..{patch_end:#x} is not contained in a file-backed LOAD segment"
            )
            .into());
        }

        let mut elf = self.into_heads().elf;
        elf.patch_address(vaddr, patch);
        Ok(Self::from(elf))
    }

    pub fn find_insert_vaddr(
        &self,
        size: u64,
        align: u64,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        if size == 0 {
            return Err("size must be greater than 0".into());
        }

        let mut load_segments: Vec<_> = self.borrow_load_segments().iter().collect();

        load_segments.sort_by_key(|segment| segment.virtual_address());

        let Some(first_segment) = load_segments.first() else {
            return Ok(0);
        };

        let mut cursor = segment_vaddr_end(first_segment)?;

        for next_segment in load_segments.iter().skip(1) {
            let next_segment_start = next_segment.virtual_address();
            let next_segment_end = segment_vaddr_end(next_segment)?;

            let candidate = align_up(cursor, align)?;

            let candidate_end = candidate.checked_add(size).ok_or_else(|| {
                format!("candidate VMA overflow: candidate={candidate:#x}, size={size:#x}")
            })?;

            if candidate_end <= next_segment_start {
                return Ok(candidate);
            }

            cursor = cursor.max(next_segment_end);
        }

        align_up(cursor, align)
    }

    pub fn strip(self) -> Self {
        let mut elf = self.into_heads().elf;
        elf.strip();
        ELFParser::from(elf)
    }

    pub fn reduce_section_by_name(
        self,
        section_name: &str,
        new_size: u64,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let elf = self.into_heads().elf;
        let mut section = elf
            .section_by_name(section_name)
            .ok_or_else(|| format!("Section not found: {section_name}"))?;

        let old_size = section.size();
        if new_size > old_size {
            return Err(format!(
                    "New section size {new_size:#x} is larger than original section size {old_size:#x}; \
                     Executable section: '{}', VMA {:#x}..{:#x}, file range {:#x}..{:#x}",
                    section.name(),
                    section.virtual_address(),
                    section.virtual_address() + section.size(),
                    section.file_offset(),
                    section.file_offset() + section.size(),
                )
                .into());
        }

        let new_content = {
            let old_size_usize = usize::try_from(old_size)?;
            let new_size_usize = usize::try_from(new_size)?;
            let old_content = section.content();

            if old_size_usize > old_content.len() {
                return Err(format!(
                        "Section '{}' old_size {old_size:#x} is larger than section content length {:#x}",
                        section.name(),
                        old_content.len()
                    )
                    .into());
            }

            if new_size_usize > old_content.len() {
                return Err(format!(
                        "Section '{}' new_size {new_size:#x} is larger than section content length {:#x}",
                        section.name(),
                        old_content.len(),
                    )
                    .into());
            }

            let mut content = old_content.to_vec();

            content[new_size_usize..].fill(0);

            content
        };

        section.set_content(&new_content);
        section.set_size(new_size);

        Ok(Self::from(elf))
    }

    fn vaddr_range_overlaps_load_segment(
        &self,
        vaddr: u64,
        size: u64,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let end = vaddr
            .checked_add(size)
            .ok_or_else(|| format!("range overflow: start={vaddr:#x}, size={size:#x}"))?;

        for segment in self.borrow_load_segments() {
            let seg_start = segment.virtual_address();
            let seg_end = segment_vaddr_end(&segment)?;

            if vaddr < seg_end && seg_start < end {
                return Ok(true);
            }
        }

        Ok(false)
    }

    pub fn insert_code_segment(
        self,
        code: &[u8],
        vaddr: u64,
        align: u64,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        if code.is_empty() {
            return Err("code segment must not be empty".into());
        }

        if align > 1 && vaddr % align != 0 {
            return Err(format!("vaddr {vaddr:#x} is not aligned to {align:#x}").into());
        }

        let parser = self.prepare_for_code_segment_insert()?;
        let size = u64::try_from(code.len())?;
        let mapped_size = parser.code_segment_mapped_size(size)?;

        if parser.vaddr_range_overlaps_load_segment(vaddr, mapped_size)? {
            let end = vaddr.checked_add(mapped_size).ok_or_else(|| {
                format!("inserted code range overflow: start={vaddr:#x}, size={mapped_size:#x}")
            })?;
            return Err(format!(
                "inserted code range {:#x}..{:#x} overlaps existing LOAD segment",
                vaddr, end,
            )
            .into());
        }

        let mut elf = parser.into_heads().elf;

        let mut new_segment = lief::elf::Segment::new();
        new_segment.set_type(lief::elf::segment::Type::LOAD);
        new_segment.set_flags(lief::elf::segment::Flags::R | lief::elf::segment::Flags::X);
        new_segment.set_virtual_address(vaddr);
        new_segment.set_physical_address(vaddr);
        new_segment.set_virtual_size(mapped_size);
        new_segment.set_alignment(align);
        new_segment.set_content(code);

        let (actual_vaddr, actual_physical_size, actual_virtual_size) = {
            let added = elf
                .add_segment(&new_segment)
                .ok_or("failed to add new LOAD segment")?;
            (
                added.virtual_address(),
                added.physical_size(),
                added.virtual_size(),
            )
        };

        if actual_vaddr != vaddr {
            return Err(format!(
                "LIEF changed inserted segment VMA from {vaddr:#x} to {actual_vaddr:#x}"
            )
            .into());
        }
        if actual_physical_size > mapped_size || actual_virtual_size > mapped_size {
            return Err(format!(
                "LIEF expanded inserted segment beyond reserved range: reserved={mapped_size:#x}, \
                 file_size={actual_physical_size:#x}, mem_size={actual_virtual_size:#x}"
            )
            .into());
        }

        Ok(ELFParser::from(elf))
    }

    pub fn into_bytes(self) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        self.try_into()
    }
}
