use anyhow::Result;
use cursive::event::{Event, Key};
use cursive::traits::*;
use cursive::views::{
    Button, Dialog, EditView, LinearLayout, Panel, ResizedView, TextContent, TextView,
};
use cursive::Cursive;
use tracing::{debug, error, info};

use std::sync::{Arc, Mutex};

use crate::core::Core;

#[derive(Clone, Copy)]
enum Unit {
    Btc,
    Sats,
}

/// Convert an amount between BTC and Satoshi units
/// TODO: use bigdecimal crate
fn convert_amount(amount: f64, from: Unit, to: Unit) -> f64 {
    match (from, to) {
        (Unit::Btc, Unit::Sats) => amount * 100_000_000.0,
        (Unit::Sats, Unit::Btc) => amount / 100_000_000.0,
        _ => amount,
    }
}

/// Initialize and run the user interface
pub fn run_ui(core: Arc<Core>, balance_content: TextContent) -> Result<()> {
    info!("Initializing UI");
    let mut siv = cursive::default();
    setup_siv(&mut siv, Arc::clone(&core), balance_content);
    info!("Starting UI event loop");
    siv.run();
    info!("UI event loop ended");
    Ok(())
}

/// Set up the Cursive interface with all necessary components and callbacks
fn setup_siv(siv: &mut Cursive, core: Arc<Core>, balance_content: TextContent) {
    siv.set_autorefresh(true);
    siv.set_window_title("BTC wallet".to_string());
    siv.add_global_callback('q', |siv| {
        info!("Quit command received");
        siv.quit()
    });
    setup_menubar(siv, Arc::clone(&core));
    setup_layout(siv, core, balance_content);
    siv.add_global_callback(Event::Key(Key::Esc), |siv| siv.select_menubar());
    siv.select_menubar();
}

/// Set up the menu bar with "Send" and "Quit" options
fn setup_menubar(siv: &mut Cursive, core: Arc<Core>) {
    siv.menubar()
        .add_leaf("Send", move |siv| {
            show_send_transaction(siv, Arc::clone(&core))
        })
        .add_leaf("Quit", |siv| siv.quit());
    siv.set_autohide_menu(false);
}

/// Set up the main layout of the application
fn setup_layout(siv: &mut Cursive, core: Arc<Core>, balance_content: TextContent) {
    let instruction = TextView::new("Press Escape to select the top menu");
    let balance_panel = Panel::new(TextView::new_with_content(balance_content)).title("Balance");
    let info_layout = create_info_layout(&core);

    let layout = LinearLayout::vertical()
        .child(instruction)
        .child(balance_panel)
        .child(info_layout);

    siv.add_layer(layout);
}

/// Create the information layout containing keys and contacts
fn create_info_layout(core: &Arc<Core>) -> LinearLayout {
    let mut info_layout = LinearLayout::horizontal();

    let keys_content = core
        .config
        .my_keys
        .iter()
        .map(|key| format!("{}", key.private.display()))
        .collect::<Vec<String>>()
        .join("\n");

    info_layout.add_child(ResizedView::with_full_width(
        Panel::new(TextView::new(keys_content)).title("Your keys"),
    ));

    let contacts_content = core
        .config
        .contacts
        .iter()
        .map(|contact| contact.name.clone())
        .collect::<Vec<String>>()
        .join("\n");

    info_layout.add_child(ResizedView::with_full_width(
        Panel::new(TextView::new(contacts_content)).title("Contacts"),
    ));

    info_layout
}

/// Display the send transaction dialog
fn show_send_transaction(siv: &mut Cursive, core: Arc<Core>) {
    info!("Showing send transaction dialog");
    let unit = Arc::new(Mutex::new(Unit::Btc));

    siv.add_layer(
        Dialog::around(create_transaction_layout(Arc::clone(&unit)))
            .title("Send Transaction")
            .button("Send", move |siv| {
                send_transaction(siv, Arc::clone(&core), *unit.lock().unwrap())
            })
            .button("Cancel", |siv| {
                debug!("Transaction cancelled");
                siv.pop_layer();
            }),
    );
}

/// Create the layout for the transaction dialog
fn create_transaction_layout(unit: Arc<Mutex<Unit>>) -> LinearLayout {
    LinearLayout::vertical()
        .child(TextView::new("Recipient:"))
        .child(EditView::new().with_name("recipient"))
        .child(TextView::new("Amount:"))
        .child(EditView::new().with_name("amount"))
        .child(create_unit_layout(unit))
}

/// Create the layout for selecting the transaction unit (BTC or Sats)
fn create_unit_layout(unit: Arc<Mutex<Unit>>) -> LinearLayout {
    LinearLayout::horizontal()
        .child(TextView::new("Unit: "))
        .child(TextView::new_with_content(TextContent::new("BTC")).with_name("unit_display"))
        .child(Button::new("Switch", move |siv| {
            switch_unit(siv, Arc::clone(&unit))
        }))
}

/// Switch the transaction unit between BTC and Sats
fn switch_unit(siv: &mut Cursive, unit: Arc<Mutex<Unit>>) {
    let mut unit = unit.lock().unwrap();
    *unit = match *unit {
        Unit::Btc => Unit::Sats,
        Unit::Sats => Unit::Btc,
    };
    siv.call_on_name("unit_display", |view: &mut TextView| {
        view.set_content(match *unit {
            Unit::Btc => "BTC",
            Unit::Sats => "Sats",
        });
    });
}

/// Process the send transaction request
fn send_transaction(siv: &mut Cursive, core: Arc<Core>, unit: Unit) {
    debug!("Send button pressed");
    let recipient = siv
        .call_on_name("recipient", |view: &mut EditView| view.get_content())
        .unwrap();
    let amount: f64 = siv
        .call_on_name("amount", |view: &mut EditView| view.get_content())
        .unwrap()
        .parse()
        .unwrap_or(0.0);

    let amount_sats = convert_amount(amount, unit, Unit::Sats) as u64;
    info!(
        "Attempting to send transaction to {} for {} sats",
        recipient, amount_sats
    );
    match core.send_transaction_async(recipient.as_str(), amount_sats) {
        Ok(_) => show_success_dialog(siv),
        Err(e) => show_error_dialog(siv, e),
    }
}

/// Display a success dialog after a successful transaction
fn show_success_dialog(siv: &mut Cursive) {
    info!("Transaction sent successfully");
    siv.add_layer(
        Dialog::text("Transaction sent successfully")
            .title("Success")
            .button("OK", |siv| {
                debug!("Closing success dialog");
                siv.pop_layer();
                siv.pop_layer();
            }),
    );
}

/// Display an error dialog when a transaction fails
fn show_error_dialog(siv: &mut Cursive, error: impl std::fmt::Display) {
    error!("Failed to send transaction: {}", error);
    siv.add_layer(
        Dialog::text(format!("Failed to send transaction: {}", error))
            .title("Error")
            .button("OK", |siv| {
                debug!("Closing error dialog");
                siv.pop_layer();
            }),
    );
}

/// Get big string with BTC balance
pub fn big_mode_btc(core: &Core) -> String {
    let btc = format!("{} BTC", btclib::sats_to_btc(core.get_balance()));
    text_to_ascii_art::to_art(btc, "small", 1, 1, 1).unwrap()
}
