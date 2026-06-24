mod commands;
// Model-agnostic AI advisor layer (GitHub Models / Ollama transport). The grounded-context
// builder and Tauri commands live in `commands::ai`.
mod ai;
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
            commands::accounts::update_account_currency,
            commands::net_worth::get_net_worth,
            commands::net_worth::get_net_worth_history,
            commands::net_worth::get_net_worth_delta,
            commands::net_worth::backfill_net_worth_history,
            commands::goals::get_goal_progress,
            commands::goals::set_goal_target,
            commands::cashflow::get_cashflow_summary,
            commands::cashflow::list_recent_transactions,
            commands::cashflow::set_transaction_flow,
            commands::cashflow::list_txn_rules,
            commands::cashflow::add_txn_rule,
            commands::cashflow::delete_txn_rule,
            commands::simulator::get_seattle_projection,
            commands::simulator::set_seattle_assumptions,
            commands::import::import_data,
            commands::fx::get_fx_rates,
            commands::fx::refresh_fx_rates,
            commands::fx::refresh_fx_rates_if_stale,
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
            commands::questrade::questrade_get_status,
            commands::questrade::questrade_connect,
            commands::questrade::questrade_sync,
            commands::questrade::questrade_disconnect,
            commands::ai::ai_get_settings,
            commands::ai::ai_save_settings,
            commands::ai::ai_set_github_token,
            commands::ai::ai_list_models,
            commands::ai::ai_chat,
        ])
        .run(tauri::generate_context!())
        .expect("TrueNorth failed to start");
}
