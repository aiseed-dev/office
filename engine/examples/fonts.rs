fn main() {
    let all = kumihan::font::list();
    println!("見つかった書体: {}", all.len());
    for f in all.iter().filter(|f| f.japanese).take(25) {
        println!("  {:<34} {}", f.name, f.path.file_name().unwrap().to_string_lossy());
    }
}
