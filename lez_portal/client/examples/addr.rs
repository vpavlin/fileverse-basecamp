//! Prints the account of one of the portal program's derived actors, in base58.
//!
//! ```text
//! addr <PROGRAM> registry
//! addr <PROGRAM> all <N>
//! addr <PROGRAM> owner <OWNER>
//! addr <PROGRAM> owned <OWNER> <I>
//! addr <PROGRAM> portal <OWNER> <I>
//! addr <PROGRAM> file <PORTAL> <INDEX>
//! addr <PROGRAM> appfile <PORTAL> <APP_FILE_ID>
//! ```
//!
//! Accounts are base58 (or hex) ids; `PROGRAM` is the address `spel deploy` printed.

use std::{process, str::FromStr};

use lee_core::account::AccountId;
use portal_client::PortalClient;

fn account(arg: Option<&String>, what: &str) -> AccountId {
    let arg = arg.unwrap_or_else(|| fail(&format!("missing {what}")));
    AccountId::from_str(arg).unwrap_or_else(|e| fail(&format!("bad {what} '{arg}': {e}")))
}

fn number(arg: Option<&String>, what: &str) -> u64 {
    let arg = arg.unwrap_or_else(|| fail(&format!("missing {what}")));
    arg.parse()
        .unwrap_or_else(|e| fail(&format!("bad {what} '{arg}': {e}")))
}

fn fail(message: &str) -> ! {
    eprintln!(
        "error: {message}\nusage: addr <PROGRAM> registry|all|owner|owned|portal|file|appfile ..."
    );
    process::exit(1);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let client = PortalClient::new(account(args.first(), "program"));
    let a = client.addresses();
    let id = match args.get(1).map(String::as_str) {
        Some("registry") => a.registry(),
        Some("all") => a.all_entry(number(args.get(2), "n")),
        Some("owner") => a.owner_head(account(args.get(2), "owner")),
        Some("owned") => a.owner_entry(account(args.get(2), "owner"), number(args.get(3), "i")),
        Some("portal") => a.portal(account(args.get(2), "owner"), number(args.get(3), "i")),
        Some("file") => a.file(account(args.get(2), "portal"), number(args.get(3), "index")),
        Some("appfile") => a.app_file(
            account(args.get(2), "portal"),
            args.get(3).unwrap_or_else(|| fail("missing app file id")),
        ),
        _ => fail("unknown address kind"),
    };
    println!("{id}");
}
