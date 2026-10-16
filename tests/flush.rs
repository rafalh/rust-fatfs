use fatfs::{FileSystem, FsOptions, StdIoWrapper};
use std::cell::RefCell;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::rc::Rc;

type DeviceState = (Cursor<Vec<u8>>, usize, bool);
#[derive(Clone)]
struct Device(Rc<RefCell<DeviceState>>);
impl Read for Device {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        self.0.borrow_mut().0.read(b)
    }
}
impl Seek for Device {
    fn seek(&mut self, p: SeekFrom) -> io::Result<u64> {
        self.0.borrow_mut().0.seek(p)
    }
}
impl Write for Device {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().0.write(b)
    }
    fn flush(&mut self) -> io::Result<()> {
        let mut s = self.0.borrow_mut();
        s.1 += 1;
        if s.2 {
            Err(io::Error::new(io::ErrorKind::Other, "flush failed"))
        } else {
            Ok(())
        }
    }
}
#[test]
fn flush_keeps_mount_writable_and_dirty_until_unmount() {
    let bytes = std::fs::read("resources/fat32.img").unwrap();
    let device = Device(Rc::new(RefCell::new((Cursor::new(bytes), 0, false))));
    let fs = FileSystem::new(StdIoWrapper::new(device.clone()), FsOptions::new()).unwrap();
    for name in ["first.txt", "second.txt"] {
        {
            let mut f = fs.root_dir().create_file(name).unwrap();
            f.write_all(b"live mount").unwrap();
            f.flush().unwrap();
        }
        let flushes = device.0.borrow().1;
        fs.flush().unwrap();
        assert_eq!(device.0.borrow().1, flushes + 1);
        assert_ne!(
            device.0.borrow().0.get_ref()[0x41] & 1,
            0,
            "flush must retain mounted dirty flag"
        );
        let mut bytes = Vec::new();
        fs.root_dir().open_file(name).unwrap().read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"live mount");
        // The FAT32 FSInfo free count was persisted without releasing the mount.
        let data = device.0.borrow();
        let fsinfo = u16::from_le_bytes(data.0.get_ref()[48..50].try_into().unwrap()) as usize * 512;
        assert_eq!(
            u32::from_le_bytes(data.0.get_ref()[fsinfo + 488..fsinfo + 492].try_into().unwrap()),
            fs.stats().unwrap().free_clusters()
        );
    }
    device.0.borrow_mut().2 = true;
    assert!(fs.flush().is_err(), "storage flush failure must be propagated");
    device.0.borrow_mut().2 = false;
    fs.unmount().unwrap();
    assert_eq!(device.0.borrow().0.get_ref()[0x41] & 1, 0);
}
