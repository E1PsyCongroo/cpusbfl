use std;

use lief::{
    self,
    generic::{Binary, Section},
};
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
    pub load_segments: Vec<lief::elf::Segment<'this>>,
}

impl From<lief::elf::Binary> for ELFParser {
    fn from(elf: lief::elf::Binary) -> Self {
        ELFParserBuilder {
            elf,
            executable_sections_builder: |elf| {
                elf.sections()
                    .filter(|s| {
                        s.flags().contains(lief::elf::section::Flags::EXECINSTR)
                            && matches!(s.get_type(), lief::elf::section::Type::PROGBITS)
                    })
                    .collect()
            },
            load_segments_builder: |elf| {
                elf.segments()
                    .filter(|s| matches!(s.p_type(), lief::elf::segment::Type::LOAD))
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

    pub fn section_containing_vma<'a>(
        &'a self,
        vma_start: u64,
        vma_end: u64,
    ) -> Option<&'a lief::elf::Section<'a>> {
        self.borrow_executable_sections()
            .iter()
            .find(|s| section_contains_range(s, vma_start, vma_end))
    }

    pub fn vma2offset(&self, vma_start: u64, vma_end: u64) -> Option<u64> {
        let section = self.section_containing_vma(vma_start, vma_end)?;
        vma_start
            .checked_sub(section.virtual_address())?
            .checked_add(section.offset())
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
                    section.offset(),
                    section.offset() + section.size(),
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

        let size = u64::try_from(code.len())?;

        if self.vaddr_range_overlaps_load_segment(vaddr, size)? {
            return Err(format!(
                "inserted code range {:#x}..{:#x} overlaps existing LOAD segment",
                vaddr,
                vaddr + size,
            )
            .into());
        }

        let mut elf = self.into_heads().elf;

        let mut new_segment = lief::elf::Segment::new();
        new_segment.set_type(lief::elf::segment::Type::LOAD);
        new_segment.set_flags(
            lief::elf::segment::Flags::W
                | lief::elf::segment::Flags::R
                | lief::elf::segment::Flags::X,
        );
        new_segment.set_virtual_address(vaddr);
        new_segment.set_physical_address(vaddr);
        new_segment.set_virtual_size(size);
        new_segment.set_alignment(align);
        new_segment.set_content(code);

        elf.add_segment(&new_segment)
            .ok_or("failed to add new LOAD segment")?;

        Ok(ELFParser::from(elf))
    }

    pub fn into_bytes(self) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        self.try_into()
    }
}
