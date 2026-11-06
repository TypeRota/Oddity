//! 离线检查当前发行物的游戏认证配置, 不读账号、不联网、不启动Windows组件.
fn main() {
    let Some(root) = std::env::args_os().nth(1) else {
        eprintln!("用法: game_auth_check <官方组件目录>");
        std::process::exit(2);
    };
    if let Err(error) = nte_core::game_auth::Client::load(std::path::Path::new(&root)) {
        eprintln!("{error}");
        std::process::exit(1);
    }
    println!("游戏认证配置校验通过. 未读取账号、联网或启动游戏.");
}
