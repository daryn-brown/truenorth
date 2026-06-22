mod commands;
// Connector groundwork shared by Phase 2+ providers. The SnapTrade connector under
// `connector::snaptrade` is exercised at runtime via `commands::snaptrade`; the trait
// scaffolding and other providers are kept ahead of use.
#[allow(dead_code)]
mod connector;
mod db;
mod fx;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            // Desktop auto-update: check GitHub Releases for a newer signed build and relaunch
            // after install. Guarded to desktop since the updater plugin is desktop-only.
            #[cfg(desktop)]
            {
                app.handle()
                    .plugin(tauri_plugin_updater::Builder::new().build())?;
                app.handle().plugin(tauri_plugin_process::init())?;
            }
            db::setup_database(app).map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            app.manage(connector::ConnectorRegistry::new());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::accounts::list_accounts,
            commands::accounts::add_account,
            commands::accounts::delete_account,
            commands::accounts::add_balance_snapshot,
            commands::net_worth::get_net_worth,
            commands::net_worth::get_net_worth_history,
            commands::net_worth::get_net_worth_delta,
            commands::goals::get_goal_progress,
            commands::goals::set_goal_target,
            commands::simulator::get_seattle_projection,
            commands::simulator::set_seattle_assumptions,
            commands::import::import_data,
            commands::fx::get_fx_rates,
            commands::fx::refresh_fx_rates,
            commands::snaptrade::snaptrade_get_status,
            commands::snaptrade::snaptrade_save_credentials,
            commands::snaptrade::snaptrade_list_users,
            commands::snaptrade::snaptrade_link_user,
            commands::snaptrade::snaptrade_get_login_link,
            commands::snaptrade::snaptrade_sync,
            commands::snaptrade::snaptrade_disconnect,
            commands::simplefin::simplefin_get_status,
            commands::simplefin::simplefin_connect,
            commands::simplefin::simplefin_sync,
            commands::simplefin::simplefin_disconnect,
        ])
        .run(tauri::generate_context!())
        .expect("TrueNorth failed to start");
}
