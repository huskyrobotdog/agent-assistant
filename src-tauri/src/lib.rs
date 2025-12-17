mod agent;

use tauri::Manager;

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_sql::Builder::new().build())
        .setup(|app| {
            let app_data = app.path().app_data_dir()?;
            let models_dir = app_data.join("models");
            std::fs::create_dir_all(&models_dir)?;
            #[cfg(debug_assertions)]
            {
                println!("模型目录: {:?}", models_dir);
            }

            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![greet])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
