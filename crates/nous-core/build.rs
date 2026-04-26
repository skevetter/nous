fn main() {
    let sqlite_include = std::env::var("DEP_SQLITE3_INCLUDE")
        .expect("DEP_SQLITE3_INCLUDE not set — libsqlite3-sys must be a direct dependency");

    cc::Build::new()
        .file("vendor/sqlite-vec/sqlite-vec.c")
        .include(&sqlite_include)
        .define("SQLITE_CORE", None)
        .define("SQLITE_VEC_STATIC", None)
        .warnings(false)
        .compile("sqlite_vec");

    println!("cargo:rerun-if-changed=vendor/sqlite-vec/sqlite-vec.c");
    println!("cargo:rerun-if-changed=vendor/sqlite-vec/sqlite-vec.h");
}
