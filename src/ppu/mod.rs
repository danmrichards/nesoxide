use crate::cartridge::Mirroring;
use registers::addr::Addr;
use registers::control::Control;
use registers::mask::Mask;
use registers::scroll::Scroll;
use registers::status::Status;

pub mod registers;

// Represents the NES PPU.
pub struct NESPPU {
    // Character (visuals) ROM.
    pub chr_rom: Vec<u8>,

    // Internal reference to colour palettes.
    pub palette_table: [u8; 32],

    // Video RAM.
    pub vram: [u8; 2048],

    // Object attribute memory (sprites).
    pub oam_addr: u8,
    pub oam_data: [u8; 256],

    pub mirroring: Mirroring,

    // Registers.
    pub addr: Addr,
    pub ctrl: Control,
    pub mask: Mask,
    pub scroll: Scroll,
    pub status: Status,

    // Is the NMI interrupt set?
    pub nmi_interrupt: Option<bool>,

    // Buffer for data read from previous request.
    buf: u8,

    // Current picture scan line
    scanline: u16,

    // Number of cycles.
    cycles: usize,
}

pub trait PPU {
    fn write_addr(&mut self, value: u8);
    fn write_ctrl(&mut self, value: u8);
    fn write_mask(&mut self, value: u8);
    fn write_scroll(&mut self, value: u8);
    fn write_data(&mut self, value: u8);
    fn write_oam_addr(&mut self, value: u8);
    fn write_oam_data(&mut self, value: u8);
    fn write_oam_dma(&mut self, value: &[u8; 256]);
    fn read_data(&mut self) -> u8;
    fn read_status(&mut self) -> u8; 
    fn read_oam_data(&self) -> u8;
}

impl NESPPU {
    // Returns an instantiated PPU.
    pub fn new(chr_rom: Vec<u8>, mirroring: Mirroring) -> Self {
        NESPPU {
            chr_rom: chr_rom,
            palette_table: [0; 32],
            vram: [0; 2048],
            oam_addr: 0,
            oam_data: [0; 64 * 4],
            mirroring: mirroring,
            buf: 0,
            addr: Addr::new(),
            ctrl: Control::new(),
            mask: Mask::new(),
            scroll: Scroll::new(),
            status: Status::new(),
            scanline: 0,
            cycles: 0,
            nmi_interrupt: None,
        }
    }


    // Returns an instatiated PPU with an empty ROM loaded.
    pub fn new_empty_rom() -> Self {
        NESPPU::new(vec![0; 2048], Mirroring::Horizontal)
    }
    
    // Increment the VRAM address based on the control register status.
    fn increment_vram_addr(&mut self) {
        self.addr.increment(self.ctrl.vram_addr_increment());
    }

    // Horizontal:
    //   [ A ] [ a ]
    //   [ B ] [ b ]
    //
    // Vertical:
    //   [ A ] [ B ]
    //   [ a ] [ b ]
    fn mirror_vram_addr(&self, addr: u16) -> u16 {
        // Mirror down 0x3000-0x3EFF to 0x2000 - 0x2EFF
        let mirrored_vram = addr & 0b1011111_1111111;
        
        // To VRAM vector.
        let vram_index = mirrored_vram - 0x2000;
        let name_table = vram_index / 0x400;
        
        match (&self.mirroring, name_table) {
            (Mirroring::Vertical, 2) | (Mirroring::Vertical, 3) => vram_index - 0x800,
            (Mirroring::Horizontal, 2) => vram_index - 0x400,
            (Mirroring::Horizontal, 1) => vram_index - 0x400,
            (Mirroring::Horizontal, 3) => vram_index - 0x800,
            _ => vram_index,
        }
    }

    // Returns true if a frame has been completed, while incrementing the cycle
    // count and scanline as appropriate.
    pub fn tick(&mut self, cycles: u8) -> bool {
        self.cycles += cycles as usize;

        // Each scanline lasts for 341 PPU clock cycles.
        if self.cycles < 341 {
            return false
        }

        self.cycles -= 341;
        self.scanline += 1;

        // println!("{}", self.scanline);

        // VBLANK is triggered at scanline 241.
        if self.scanline == 241 {
            self.status.set_vblank_status(true);
            self.status.set_sprite_zero_hit(false);

            // Set the interrupt if the control register allows it.
            if self.ctrl.vblank_nmi() {
                self.nmi_interrupt = Some(true);
            }
        } else if self.scanline >= 262 {
            // There are 262 scanlines per frame.
            self.scanline = 0;
            self.nmi_interrupt = None;
            self.status.set_sprite_zero_hit(false);
            self.status.reset_vblank_status();
            return true;
        }
        
        return false
    }
}

impl PPU for NESPPU {
    // Writes value to the address register.
    fn write_addr(&mut self, value: u8) {
        self.addr.update(value);
    }

    // Writes to the control register.
    fn write_ctrl(&mut self, value: u8) {
        let start_nmi = self.ctrl.vblank_nmi();
        
        self.ctrl.update(value);

        if !start_nmi && self.ctrl.vblank_nmi() && self.status.is_in_vblank() {
            self.nmi_interrupt = Some(true);
        }
    }

    // Writes to the mask register.
    fn write_mask(&mut self, value: u8) {
        self.mask.update(value);
    }

    // Writes to the scroll register.
    fn write_scroll(&mut self, value: u8) {
        self.scroll.write(value);
    }

    fn write_oam_addr(&mut self, value: u8) {
        self.oam_addr = value;
    }

    fn write_oam_data(&mut self, value: u8) {
        self.oam_data[self.oam_addr as usize] = value;
        self.oam_addr = self.oam_addr.wrapping_add(1);
    }

    fn write_oam_dma(&mut self, data: &[u8; 256]) {
        for x in data.iter() {
            self.oam_data[self.oam_addr as usize] = *x;
            self.oam_addr = self.oam_addr.wrapping_add(1);
        }
    }

    // Writes data to appropriate location based on the address register.
    fn write_data(&mut self, value: u8) {
        let addr = self.addr.get();
        match addr {
            0..=0x1FFF => println!("attempt to write to chr rom space {}", addr), 
            0x2000..=0x2FFF => {
                self.vram[self.mirror_vram_addr(addr) as usize] = value;
            }
            0x3000..=0x3eff => unimplemented!("addr {} shouldn't be used in reallity", addr),

            // Addresses $3F10/$3F14/$3F18/$3F1C are mirrors of
            // $3F00/$3F04/$3F08/$3F0C
            0x3F10 | 0x3F14 | 0x3F18 | 0x3F1C => {
                let add_mirror = addr - 0x10;
                self.palette_table[(add_mirror - 0x3F00) as usize] = value;
            }
            0x3F00..=0x3FFF =>
            {
                self.palette_table[(addr - 0x3F00) as usize] = value;
            }
            _ => panic!("unexpected access to mirrored space {}", addr),
        }
        self.increment_vram_addr();
    }

    // Retuns data from appropriate source based on the address register.
    fn read_data(&mut self) -> u8 {
        let addr = self.addr.get();
        self.increment_vram_addr();
 
        match addr {
            0..=0x1FFF => {
                let result = self.buf;
                self.buf = self.chr_rom[addr as usize];
                result
            },
            0x2000..=0x2FFF => {
                let result = self.buf;
                self.buf = self.vram[self.mirror_vram_addr(addr) as usize];
                result
            },
            0x3000..=0x3EFF => panic!("addr space 0x3000..0x3EFF is not expected to be used, requested = {} ", addr),
            0x3F00..=0x3FFF =>
            {
                self.palette_table[(addr - 0x3F00) as usize]
            }
            _ => panic!("unexpected access to mirrored space {}", addr),
        }
    }

    // Returns the PPU status register and resets VBLANK + addr.
    fn read_status(&mut self) -> u8 {
        let data = self.status.snapshot();
        self.status.reset_vblank_status();
        self.addr.reset();
        self.scroll.reset_latch();
        data
    }

    fn read_oam_data(&self) -> u8 {
        self.oam_data[self.oam_addr as usize]
    }
}

#[cfg(test)]
pub mod test {
    use super::*;

    #[test]
    fn test_ppu_vram_writes() {
        let mut ppu = NESPPU::new_empty_rom();
        ppu.write_addr(0x23);
        ppu.write_addr(0x05);
        ppu.write_data(0x66);

        assert_eq!(ppu.vram[0x0305], 0x66);
    }

    #[test]
    fn test_ppu_vram_reads() {
        let mut ppu = NESPPU::new_empty_rom();
        ppu.write_ctrl(0);
        ppu.vram[0x0305] = 0x66;

        ppu.write_addr(0x23);
        ppu.write_addr(0x05);

        ppu.read_data();
        assert_eq!(ppu.addr.get(), 0x2306);
        assert_eq!(ppu.read_data(), 0x66);
    }

    #[test]
    fn test_ppu_vram_reads_cross_page() {
        let mut ppu = NESPPU::new_empty_rom();
        ppu.write_ctrl(0);
        ppu.vram[0x01ff] = 0x66;
        ppu.vram[0x0200] = 0x77;

        ppu.write_addr(0x21);
        ppu.write_addr(0xff);

        ppu.read_data();
        assert_eq!(ppu.read_data(), 0x66);
        assert_eq!(ppu.read_data(), 0x77);
    }

    #[test]
    fn test_ppu_vram_reads_step_32() {
        let mut ppu = NESPPU::new_empty_rom();
        ppu.write_ctrl(0b100);
        ppu.vram[0x01ff] = 0x66;
        ppu.vram[0x01ff + 32] = 0x77;
        ppu.vram[0x01ff + 64] = 0x88;

        ppu.write_addr(0x21);
        ppu.write_addr(0xff);

        ppu.read_data();
        assert_eq!(ppu.read_data(), 0x66);
        assert_eq!(ppu.read_data(), 0x77);
        assert_eq!(ppu.read_data(), 0x88);
    }

    // Horizontal: https://wiki.nesdev.com/w/index.php/Mirroring
    //   [0x2000 A ] [0x2400 a ]
    //   [0x2800 B ] [0x2C00 b ]
    #[test]
    fn test_vram_horizontal_mirror() {
        let mut ppu = NESPPU::new_empty_rom();
        ppu.write_addr(0x24);
        ppu.write_addr(0x05);

        ppu.write_data(0x66);

        ppu.write_addr(0x28);
        ppu.write_addr(0x05);

        ppu.write_data(0x77);

        ppu.write_addr(0x20);
        ppu.write_addr(0x05);

        ppu.read_data();
        assert_eq!(ppu.read_data(), 0x66);

        ppu.write_addr(0x2C);
        ppu.write_addr(0x05);

        ppu.read_data();
        assert_eq!(ppu.read_data(), 0x77);
    }

    // Vertical: https://wiki.nesdev.com/w/index.php/Mirroring
    //   [0x2000 A ] [0x2400 B ]
    //   [0x2800 a ] [0x2C00 b ]
    #[test]
    fn test_vram_vertical_mirror() {
        let mut ppu = NESPPU::new(vec![0; 2048], Mirroring::Vertical);

        ppu.write_addr(0x20);
        ppu.write_addr(0x05);

        ppu.write_data(0x66);

        ppu.write_addr(0x2C);
        ppu.write_addr(0x05);

        ppu.write_data(0x77);

        ppu.write_addr(0x28);
        ppu.write_addr(0x05);

        ppu.read_data();
        assert_eq!(ppu.read_data(), 0x66);

        ppu.write_addr(0x24);
        ppu.write_addr(0x05);

        ppu.read_data();
        assert_eq!(ppu.read_data(), 0x77);
    }

    #[test]
    fn test_read_status_resets_latch() {
        let mut ppu = NESPPU::new_empty_rom();
        ppu.vram[0x0305] = 0x66;

        ppu.write_addr(0x21);
        ppu.write_addr(0x23);
        ppu.write_addr(0x05);

        ppu.read_data();
        assert_ne!(ppu.read_data(), 0x66);

        ppu.read_status();

        ppu.write_addr(0x23);
        ppu.write_addr(0x05);

        ppu.read_data();
        assert_eq!(ppu.read_data(), 0x66);
    }

    #[test]
    fn test_ppu_vram_mirroring() {
        let mut ppu = NESPPU::new_empty_rom();
        ppu.write_ctrl(0);
        ppu.vram[0x0305] = 0x66;

        ppu.write_addr(0x63);
        ppu.write_addr(0x05);

        ppu.read_data();
        assert_eq!(ppu.read_data(), 0x66);
    }

    #[test]
    fn test_read_status_resets_vblank() {
        let mut ppu = NESPPU::new_empty_rom();
        ppu.status.set_vblank_status(true);

        let status = ppu.read_status();

        assert_eq!(status >> 7, 1);
        assert_eq!(ppu.status.snapshot() >> 7, 0);
    }

    #[test]
    fn test_oam_read_write() {
        let mut ppu = NESPPU::new_empty_rom();
        ppu.write_oam_addr(0x10);
        ppu.write_oam_data(0x66);
        ppu.write_oam_data(0x77);

        ppu.write_oam_addr(0x10);
        assert_eq!(ppu.read_oam_data(), 0x66);

        ppu.write_oam_addr(0x11);
        assert_eq!(ppu.read_oam_data(), 0x77);
    }

    #[test]
    fn test_oam_dma() {
        let mut ppu = NESPPU::new_empty_rom();

        let mut data = [0x66; 256];
        data[0] = 0x77;
        data[255] = 0x88;

        ppu.write_oam_addr(0x10);
        ppu.write_oam_dma(&data);

        ppu.write_oam_addr(0xF);
        assert_eq!(ppu.read_oam_data(), 0x88);

        ppu.write_oam_addr(0x10);
        assert_eq!(ppu.read_oam_data(), 0x77);
  
        ppu.write_oam_addr(0x11);
        assert_eq!(ppu.read_oam_data(), 0x66);
    }
}