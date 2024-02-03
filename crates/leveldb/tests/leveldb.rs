use std::{
    fs::{create_dir_all, remove_dir_all},
    path::PathBuf,
};

use hala_leveldb::Database;

fn test_path() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("test");

    if path.exists() {
        remove_dir_all(path.clone()).unwrap();
    }

    create_dir_all(path.clone()).unwrap();

    path
}

#[test]
fn test_leveldb() {
    let path = test_path();

    let db = Database::open(path, None).unwrap();

    for i in 0..100 {
        db.put(format!("hello {}", i), format!("world {}", i), None)
            .unwrap();
    }

    for i in 0..100 {
        let value: String = db.get(format!("hello {}", i), None).unwrap();

        assert_eq!(value, format!("world {}", i));
    }

    assert_eq!(db.iter(None).collect::<Vec<_>>().len(), 100);

    for i in 1..10 {
        let c = db
            .iter(None)
            .seek(format!("hello {}", i))
            .map(|item| {
                (
                    item.key::<String>().unwrap(),
                    item.value::<String>().unwrap(),
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(c.len(), 99 - (i - 1) * 11)
    }
}

// #[test]
// fn test_leveldb_mem_leak() {
//     let path = test_path();

//     let db = Database::open(path, None).unwrap();

//     for i in 0..100 {
//         db.put(format!("hello {}", i), format!("world {}", i), None)
//             .unwrap();
//     }

//     for i in 0..100 {
//         let value: String = db.get(format!("hello {}", i), None).unwrap();

//         assert_eq!(value, format!("world {}", i));
//     }

//     loop {
//         for item in db.iter(None) {
//             item.key::<String>().unwrap();
//             item.value::<String>().unwrap();
//         }

//         sleep(Duration::from_millis(100));
//     }
// }
