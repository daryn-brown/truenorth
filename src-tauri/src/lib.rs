mod commands;
mod connector;
mod db;
mod fx;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            db::setup_database(app)
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::accounts::list_accounts,
            commands::accounts::add_account,
            commands::accounts::delete_account,
            commands::accounts::add_balance_snapshot,
            commands::net_worth::get_net_worth,
            commands::fx::get_fx_rates,
            commands::fx::refresh_fx_rates,
        ])
        .run(tauri::generate_context!())
        .expect("Finance Second Brain failed to start");
}

