const ADDI_OPCODE: u32 = 0x13;
const ADDIW_OPCODE: u32 = 0x1b;
const AUIPC_OPCODE: u32 = 0x17;
const JAL_OPCODE: u32 = 0x6f;
const JALR_OPCODE: u32 = 0x67;
const LUI_OPCODE: u32 = 0x37;
const AMO_OPCODE: u32 = 0x2f;
const STORE_OPCODE: u32 = 0x23;
const SYSTEM_OPCODE: u32 = 0x73;

const NOP_INSN: u32 = 0x0000_0013;
const C_NOP_INSN: u16 = 0x0001;
const MRET_INSN: u32 = 0x3020_0073;

pub(crate) const NOP: [u8; 4] = NOP_INSN.to_le_bytes();
pub(crate) const MRET: [u8; 4] = MRET_INSN.to_le_bytes();
pub(crate) const C_NOP: [u8; 2] = C_NOP_INSN.to_le_bytes();

pub(crate) const CSR_SSTATUS: u16 = 0x100;
pub(crate) const CSR_STVEC: u16 = 0x105;
pub(crate) const CSR_SEPC: u16 = 0x141;
pub(crate) const CSR_SCAUSE: u16 = 0x142;
pub(crate) const CSR_STVAL: u16 = 0x143;
pub(crate) const CSR_SSCRATCH: u16 = 0x140;
pub(crate) const CSR_SATP: u16 = 0x180;
pub(crate) const CSR_MSTATUS: u16 = 0x300;
pub(crate) const CSR_MEDELEG: u16 = 0x302;
pub(crate) const CSR_MIDELEG: u16 = 0x303;
pub(crate) const CSR_MIE: u16 = 0x304;
pub(crate) const CSR_MTVEC: u16 = 0x305;
pub(crate) const CSR_MSCRATCH: u16 = 0x340;
pub(crate) const CSR_MEPC: u16 = 0x341;
pub(crate) const CSR_MCAUSE: u16 = 0x342;
pub(crate) const CSR_MTVAL: u16 = 0x343;
pub(crate) const CSR_MIP: u16 = 0x344;

const REG_COUNT: usize = 32;
const ZERO_REG: u32 = 0;
const SP_REG: usize = 2;

pub(crate) const COMPRESSED_INST_BYTES: usize = 2;
pub(crate) const STANDARD_INST_BYTES: usize = 4;
const INST_LEN_TAG_MASK: u16 = 0b11;
const STANDARD_INST_LEN_TAG: u16 = 0b11;

const FUNCT3_ADDI: u32 = 0b000;
const FUNCT3_SLLI: u32 = 0b001;
const FUNCT3_JALR: u32 = 0b000;
const FUNCT3_CSRRW: u32 = 0b001;
const FUNCT3_STORE_BYTE: u32 = 0b000;
const FUNCT3_STORE_HALF: u32 = 0b001;
const FUNCT3_STORE_WORD: u32 = 0b010;
const FUNCT3_STORE_DOUBLE: u32 = 0b011;

const IMM12_MIN: i32 = -(1 << 11);
const IMM12_MAX: i32 = (1 << 11) - 1;
const IMM12_BITS: u32 = 12;
const IMM12_MASK: u32 = (1 << IMM12_BITS) - 1;
const IMM12_SIGN_BIAS_I64: i64 = 1 << (IMM12_BITS - 1);
const IMM12_SIGN_BIAS_I128: i128 = 1 << (IMM12_BITS - 1);
const JAL_OFFSET_MIN: i32 = -(1 << 20);
const JAL_OFFSET_MAX: i32 = (1 << 20) - 2;
const AUIPC_IMM_MIN: i32 = -(1 << 19);
const AUIPC_IMM_MAX: i32 = (1 << 19) - 1;
const UIMM20_MASK: u32 = (1 << 20) - 1;

const RVC_SP_STORE_MASK: u16 = 0xe003;
const RVC_C_SW_MATCH: u16 = 0xc000;
const RVC_C_SD_MATCH: u16 = 0xe000;
const RVC_C_SWSP_MATCH: u16 = 0xc002;
const RVC_C_SDSP_MATCH: u16 = 0xe002;

const AMO_FUNCT5_LR: u32 = 0b00010;
const AMO_FUNCT5_SC: u32 = 0b00011;
const AMO_FUNCT5_SWAP: u32 = 0b00001;
const AMO_FUNCT5_ADD: u32 = 0b00000;
const AMO_FUNCT5_XOR: u32 = 0b00100;
const AMO_FUNCT5_AND: u32 = 0b01100;
const AMO_FUNCT5_OR: u32 = 0b01000;
const AMO_FUNCT5_MIN: u32 = 0b10000;
const AMO_FUNCT5_MAX: u32 = 0b10100;
const AMO_FUNCT5_MINU: u32 = 0b11000;
const AMO_FUNCT5_MAXU: u32 = 0b11100;

#[derive(Clone, Copy)]
pub(crate) struct MemoryWrite {
    addr: u64,
    value: u64,
    width: StoreWidth,
}

#[derive(Clone, Copy)]
enum StoreWidth {
    Byte,
    Half,
    Word,
    Double,
}

pub(crate) enum MemoryWriteDecode {
    NotStore,
    Write(MemoryWrite),
    Unsupported,
}

fn riscv_inst_len(first_halfword: u16) -> usize {
    if first_halfword & INST_LEN_TAG_MASK == STANDARD_INST_LEN_TAG {
        STANDARD_INST_BYTES
    } else {
        COMPRESSED_INST_BYTES
    }
}

pub(crate) fn inst_len_at(input: &[u8], offset: usize) -> usize {
    assert!(offset + 2 <= input.len());

    let halfword = u16::from_le_bytes([input[offset], input[offset + 1]]);
    let inst_len = riscv_inst_len(halfword);
    assert!(offset + inst_len <= input.len());

    inst_len
}

fn push_inst(output: &mut Vec<u8>, inst: u32) {
    output.extend_from_slice(&inst.to_le_bytes());
}

fn encode_i_type(imm12: u32, rs1: u8, funct3: u32, rd: u8, opcode: u32) -> u32 {
    assert!(imm12 & IMM12_MASK == imm12);

    imm12 << 20 | (rs1 as u32) << 15 | (funct3 as u32) << 12 | (rd as u32) << 7 | opcode
}

fn encode_u_type(imm20: u32, rd: u8, opcode: u32) -> u32 {
    assert!(imm20 & UIMM20_MASK == imm20);

    imm20 << 12 | (rd as u32) << 7 | opcode
}

fn encode_addi(rd: u8, rs1: u8, imm: u32) -> u32 {
    encode_i_type(imm, rs1, FUNCT3_ADDI, rd, ADDI_OPCODE)
}

fn encode_addiw(rd: u8, rs1: u8, imm: u32) -> u32 {
    encode_i_type(imm, rs1, FUNCT3_ADDI, rd, ADDIW_OPCODE)
}

fn encode_slli(rd: u8, rs1: u8, shamt: u8) -> u32 {
    let shamt = u32::from(shamt);
    assert!(shamt < 64);
    encode_i_type(shamt, rs1, FUNCT3_SLLI, rd, ADDI_OPCODE)
}

fn encode_lui(rd: u8, imm20: u32) -> u32 {
    encode_u_type(imm20, rd, LUI_OPCODE)
}

fn encode_auipc(rd: u8, imm20: u32) -> u32 {
    encode_u_type(imm20, rd, AUIPC_OPCODE)
}

fn encode_jal(rd: u8, imm: u32) -> u32 {
    (((imm >> 20) & 0x1) << 31)
        | (((imm >> 1) & 0x3ff) << 21)
        | (((imm >> 11) & 0x1) << 20)
        | (((imm >> 12) & 0xff) << 12)
        | ((rd as u32) << 7)
        | JAL_OPCODE
}

fn encode_jalr(rd: u8, rs1: u8, imm: u32) -> u32 {
    encode_i_type(imm, rs1, FUNCT3_JALR, rd, JALR_OPCODE)
}

fn encode_store(rs1: u8, rs2: u8, imm: u32, funct3: u32) -> u32 {
    assert!(imm & IMM12_MASK == imm);
    let imm_11_5 = (imm >> 5) & 0x7f;
    let imm_4_0 = imm & 0x1f;

    imm_11_5 << 25
        | (rs2 as u32) << 20
        | (rs1 as u32) << 15
        | funct3 << 12
        | imm_4_0 << 7
        | STORE_OPCODE
}

fn encode_csrrw(csr: u16, rs1: u8) -> u32 {
    (csr as u32) << 20 | (rs1 as u32) << 15 | FUNCT3_CSRRW << 12 | SYSTEM_OPCODE
}

fn sign_extend(value: u64, bits: u32) -> u64 {
    assert!((1..=64).contains(&bits));
    if bits == 64 {
        value
    } else {
        let shift = 64 - bits;
        (((value << shift) as i64) >> shift) as u64
    }
}

fn is_int_n(x: u64, bits: u32) -> bool {
    assert!((1..=64).contains(&bits));
    x == sign_extend(x, bits)
}

fn imm12(value: u64) -> u32 {
    (value as u32) & IMM12_MASK
}

fn encode_signed_imm12(value: i128) -> u32 {
    assert!((i128::from(IMM12_MIN)..=i128::from(IMM12_MAX)).contains(&value));
    (value as i32 as u32) & IMM12_MASK
}

pub(crate) fn append_load_u64(output: &mut Vec<u8>, rd: u8, value: u64) -> u64 {
    let mut append_inst_count = 0;
    if is_int_n(value, 32) {
        let lo12 = imm12(value);
        let hi20 = (((value as i64 + IMM12_SIGN_BIAS_I64) >> IMM12_BITS) as u32) & UIMM20_MASK;
        if hi20 != 0 {
            push_inst(output, encode_lui(rd, hi20));
            append_inst_count += 1;
            if lo12 != 0 {
                push_inst(output, encode_addiw(rd, rd, lo12));
                append_inst_count += 1;
            }
        } else {
            push_inst(output, encode_addi(rd, 0, lo12));
            append_inst_count += 1;
        }
        append_inst_count
    } else {
        let lo12 = imm12(value);
        let hi52 = value.wrapping_add(IMM12_SIGN_BIAS_I64 as u64) >> IMM12_BITS;

        let tz = hi52.trailing_zeros() as u32;
        let shift_amount = IMM12_BITS + tz;

        let shifted_hi = hi52 >> tz;
        let hi_bits = 64 - shift_amount;
        let hi = sign_extend(shifted_hi, hi_bits);

        append_inst_count += append_load_u64(output, rd, hi);
        push_inst(output, encode_slli(rd, rd, shift_amount as u8));
        append_inst_count += 1;

        if lo12 != 0 {
            push_inst(output, encode_addi(rd, rd, lo12));
            append_inst_count += 1;
        }

        append_inst_count
    }
}

pub(crate) fn append_write_csr(output: &mut Vec<u8>, csr: u16, value: u64, scratch_reg: u8) -> u64 {
    let append_inst_count = append_load_u64(output, scratch_reg, value);
    push_inst(output, encode_csrrw(csr, scratch_reg));
    append_inst_count + 1
}

pub(crate) fn append_memory_restore(
    output: &mut Vec<u8>,
    memory_writes: &[MemoryWrite],
    addr_reg: u8,
    value_reg: u8,
) -> u64 {
    let mut append_inst_count = 0;
    for write in memory_writes {
        append_inst_count += append_load_u64(output, addr_reg, write.addr);
        append_inst_count += append_load_u64(output, value_reg, write.value);
        push_inst(
            output,
            encode_store(addr_reg, value_reg, 0, write.width.store_funct3()),
        );
        append_inst_count += 1;
    }

    append_inst_count
}

fn jmp_reg(reg: Option<u64>) -> Option<u8> {
    let reg = reg.unwrap_or(1);
    if (1..=31).contains(&reg) {
        Some(reg as u8)
    } else {
        None
    }
}

pub(crate) fn encode_jmp(
    from_pc: u64,
    to_pc: u64,
    longjmp: bool,
    reg: Option<u64>,
) -> Option<Vec<u8>> {
    let offset = i128::from(to_pc) - i128::from(from_pc);
    assert!(offset % 2 == 0);

    if (i128::from(JAL_OFFSET_MIN)..=i128::from(JAL_OFFSET_MAX)).contains(&offset) {
        let inst = encode_jal(0, offset as u32);
        Some(inst.to_le_bytes().to_vec())
    } else if longjmp {
        let reg = jmp_reg(reg)?;
        let hi20 = (offset + IMM12_SIGN_BIAS_I128) >> IMM12_BITS;
        if !(i128::from(AUIPC_IMM_MIN)..=i128::from(AUIPC_IMM_MAX)).contains(&hi20) {
            return None;
        }
        let base = hi20 << IMM12_BITS;
        let lo12 = offset.checked_sub(base)?;
        if !(i128::from(IMM12_MIN)..=i128::from(IMM12_MAX)).contains(&lo12) {
            return None;
        }

        let mut bytes = Vec::with_capacity(8);
        bytes.extend_from_slice(
            &encode_auipc(reg, (hi20 as i32 as u32) & UIMM20_MASK).to_le_bytes(),
        );
        bytes.extend_from_slice(&encode_jalr(0, reg, encode_signed_imm12(lo12)).to_le_bytes());
        Some(bytes)
    } else {
        None
    }
}

impl StoreWidth {
    fn bytes(self) -> u64 {
        match self {
            StoreWidth::Byte => 1,
            StoreWidth::Half => 2,
            StoreWidth::Word => 4,
            StoreWidth::Double => 8,
        }
    }

    fn mask(self) -> u64 {
        match self {
            StoreWidth::Double => u64::MAX,
            _ => (1 << (self.bytes() * 8)) - 1,
        }
    }

    fn store_funct3(self) -> u32 {
        match self {
            StoreWidth::Byte => FUNCT3_STORE_BYTE,
            StoreWidth::Half => FUNCT3_STORE_HALF,
            StoreWidth::Word => FUNCT3_STORE_WORD,
            StoreWidth::Double => FUNCT3_STORE_DOUBLE,
        }
    }

    fn truncate(self, value: u64) -> u64 {
        value & self.mask()
    }
}

fn compressed_reg(reg: u16) -> usize {
    (reg as usize) + 8
}

fn memory_write(addr: u64, value: u64, width: StoreWidth) -> MemoryWrite {
    MemoryWrite {
        addr,
        value: width.truncate(value),
        width,
    }
}

fn decode_s_type_imm(inst: u32) -> i64 {
    let imm = (inst >> 25) << 5 | (inst >> 7) & 0x1f;
    sign_extend(imm as u64, IMM12_BITS) as i64
}

fn decode_standard_store(inst: u32, regs: &[u64; REG_COUNT]) -> MemoryWriteDecode {
    let opcode = inst & 0x7f;
    if opcode == STORE_OPCODE {
        let funct3 = inst >> 12 & 0x7;
        let width = match funct3 {
            FUNCT3_STORE_BYTE => StoreWidth::Byte,
            FUNCT3_STORE_HALF => StoreWidth::Half,
            FUNCT3_STORE_WORD => StoreWidth::Word,
            FUNCT3_STORE_DOUBLE => StoreWidth::Double,
            _ => return MemoryWriteDecode::Unsupported,
        };
        let rs1 = inst >> 15 & 0x1f;
        let rs2 = inst >> 20 & 0x1f;
        let addr = regs[rs1 as usize].wrapping_add(decode_s_type_imm(inst) as u64);
        return MemoryWriteDecode::Write(memory_write(addr, regs[rs2 as usize], width));
    }

    if opcode == AMO_OPCODE {
        return decode_atomic_memory_write(inst, regs);
    }

    MemoryWriteDecode::NotStore
}

fn decode_atomic_memory_write(inst: u32, regs: &[u64; REG_COUNT]) -> MemoryWriteDecode {
    let funct3 = inst >> 12 & 0x7;
    let width = match funct3 {
        FUNCT3_STORE_WORD => StoreWidth::Word,
        FUNCT3_STORE_DOUBLE => StoreWidth::Double,
        _ => return MemoryWriteDecode::NotStore,
    };
    let rd = inst >> 7 & 0x1f;
    let rs1 = inst >> 15 & 0x1f;
    let rs2 = inst >> 20 & 0x1f;
    let funct5 = inst >> 27 & 0x1f;
    let addr = regs[rs1 as usize];
    let rhs = width.truncate(regs[rs2 as usize]);

    let value = match funct5 {
        AMO_FUNCT5_LR => return MemoryWriteDecode::NotStore,
        AMO_FUNCT5_SC => {
            if rd == ZERO_REG {
                return MemoryWriteDecode::Unsupported;
            }
            if regs[rd as usize] == 0 {
                rhs
            } else {
                return MemoryWriteDecode::NotStore;
            }
        }
        AMO_FUNCT5_SWAP => rhs,
        _ => {
            if rd == ZERO_REG {
                return MemoryWriteDecode::Unsupported;
            }
            let old = width.truncate(regs[rd as usize]);
            match funct5 {
                AMO_FUNCT5_ADD => old.wrapping_add(rhs),
                AMO_FUNCT5_XOR => old ^ rhs,
                AMO_FUNCT5_AND => old & rhs,
                AMO_FUNCT5_OR => old | rhs,
                AMO_FUNCT5_MIN => signed_min(old, rhs, width),
                AMO_FUNCT5_MAX => signed_max(old, rhs, width),
                AMO_FUNCT5_MINU => old.min(rhs),
                AMO_FUNCT5_MAXU => old.max(rhs),
                _ => return MemoryWriteDecode::Unsupported,
            }
        }
    };

    MemoryWriteDecode::Write(memory_write(addr, value, width))
}

fn signed_min(lhs: u64, rhs: u64, width: StoreWidth) -> u64 {
    let bits = (width.bytes() * 8) as u32;
    if sign_extend(lhs, bits) as i64 <= sign_extend(rhs, bits) as i64 {
        lhs
    } else {
        rhs
    }
}

fn signed_max(lhs: u64, rhs: u64, width: StoreWidth) -> u64 {
    let bits = (width.bytes() * 8) as u32;
    if sign_extend(lhs, bits) as i64 >= sign_extend(rhs, bits) as i64 {
        lhs
    } else {
        rhs
    }
}

fn decode_compressed_store(inst: u16, regs: &[u64; REG_COUNT]) -> MemoryWriteDecode {
    match inst & RVC_SP_STORE_MASK {
        RVC_C_SW_MATCH => {
            let rs1 = compressed_reg(inst >> 7 & 0x7);
            let rs2 = compressed_reg(inst >> 2 & 0x7);
            let imm =
                ((inst >> 10) & 0x7) << 3 | ((inst >> 6) & 0x1) << 2 | ((inst >> 5) & 0x1) << 6;
            let addr = regs[rs1].wrapping_add(imm as u64);
            MemoryWriteDecode::Write(memory_write(addr, regs[rs2], StoreWidth::Word))
        }
        RVC_C_SD_MATCH => {
            let rs1 = compressed_reg(inst >> 7 & 0x7);
            let rs2 = compressed_reg(inst >> 2 & 0x7);
            let imm = ((inst >> 10) & 0x7) << 3 | ((inst >> 5) & 0x3) << 6;
            let addr = regs[rs1].wrapping_add(imm as u64);
            MemoryWriteDecode::Write(memory_write(addr, regs[rs2], StoreWidth::Double))
        }
        RVC_C_SWSP_MATCH => {
            let rs2 = (inst >> 2 & 0x1f) as usize;
            let imm = ((inst >> 9) & 0xf) << 2 | ((inst >> 7) & 0x3) << 6;
            let addr = regs[SP_REG].wrapping_add(imm as u64);
            MemoryWriteDecode::Write(memory_write(addr, regs[rs2], StoreWidth::Word))
        }
        RVC_C_SDSP_MATCH => {
            let rs2 = (inst >> 2 & 0x1f) as usize;
            let imm = ((inst >> 10) & 0x7) << 3 | ((inst >> 7) & 0x7) << 6;
            let addr = regs[SP_REG].wrapping_add(imm as u64);
            MemoryWriteDecode::Write(memory_write(addr, regs[rs2], StoreWidth::Double))
        }
        _ => MemoryWriteDecode::NotStore,
    }
}

pub(crate) fn decode_memory_write_at(
    input: &[u8],
    offset: usize,
    regs: &[u64; REG_COUNT],
) -> MemoryWriteDecode {
    match inst_len_at(input, offset) {
        STANDARD_INST_BYTES => {
            let inst = u32::from_le_bytes([
                input[offset],
                input[offset + 1],
                input[offset + 2],
                input[offset + 3],
            ]);
            decode_standard_store(inst, regs)
        }
        COMPRESSED_INST_BYTES => {
            let inst = u16::from_le_bytes([input[offset], input[offset + 1]]);
            decode_compressed_store(inst, regs)
        }
        _ => panic!("instruction length must be 2 or 4 bytes"),
    }
}
