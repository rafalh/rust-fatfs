mod common;

use fatfs::{FatType, FormatVolumeOptions};

use crate::common::{create_temp_fs, init_logger};

#[test]
fn test_rename_dir_updates_dotdot_cluster() {
    init_logger();
    for (fat_type, total_bytes) in [
        (FatType::Fat12, 2 * 1024 * 1024),
        (FatType::Fat16, 16 * 1024 * 1024),
        (FatType::Fat32, 64 * 1024 * 1024),
    ] {
        let fs = create_temp_fs(FormatVolumeOptions::new().fat_type(fat_type), total_bytes);

        let root_dir = fs.root_dir();
        root_dir.create_dir("rename-source").unwrap();
        let destination = root_dir.create_dir("rename-destination").unwrap();

        root_dir.rename("rename-source", &destination, "moved").unwrap();
        let moved = destination.open_dir("moved").unwrap();
        let moved_parent = moved.open_dir("..").unwrap();
        assert!(moved_parent.open_dir("moved").is_ok());

        destination.rename("moved", &root_dir, "moved-back").unwrap();
        let moved_back = root_dir.open_dir("moved-back").unwrap();
        let moved_back_parent = moved_back.open_dir("..").unwrap();
        assert!(moved_back_parent.open_dir("moved-back").is_ok());
    }
}
