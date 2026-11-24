//! File and app-file actors. They only store what their portal sends: every message must
//! come from the portal the actor's address derives from.

use lee_core::{
    account::{AccountId, Actor},
    program::{ReceiveInput, Response},
};
use portal_core::{
    event::{DeletedFile, PortalEvent as _},
    AppFile, File, FileType, Internal, PortalState,
};

use crate::{addresses, assert_receiver, file_seed, send_to_pda, state};

/// The fields of a new file.
pub(crate) struct NewFile {
    pub index: u64,
    pub app_file_id: String,
    pub file_type: FileType,
    pub metadata_cid: String,
    pub content_cid: String,
    pub gate_cid: String,
    pub version: u32,
    pub editor: AccountId,
}

/// The fields an edit replaces.
pub(crate) struct FileEdit {
    pub file_type: FileType,
    pub metadata_cid: String,
    pub content_cid: String,
    pub gate_cid: String,
    pub version: u32,
    pub editor: AccountId,
}

fn file_state(input: &ReceiveInput) -> File {
    match state(input) {
        Some(PortalState::File(file)) => file,
        _ => panic!("receiver is not a file"),
    }
}

/// At a new file actor, from its portal.
pub(crate) fn write(input: &ReceiveInput, from: Actor, new: NewFile) -> Response {
    let portal = from.account_id;
    assert_receiver(input, addresses(input).file(portal, new.index), "file");
    assert!(input.pre_state.is_empty(), "file actor must be empty");
    assert!(input.is_authorized, "file creation must be authorized");
    let file = File {
        portal,
        index: new.index,
        app_file_id: new.app_file_id,
        file_type: new.file_type,
        metadata_cid: new.metadata_cid,
        content_cid: new.content_cid,
        gate_cid: new.gate_cid,
        version: new.version,
        deleted: false,
        last_editor: new.editor,
    };
    Response::set_state(PortalState::File(file).encode())
}

/// At a file actor, from its portal.
pub(crate) fn update(input: &ReceiveInput, from: Actor, edit: FileEdit) -> Response {
    let mut file = file_state(input);
    assert!(
        file.portal == from.account_id,
        "message must come from the file's portal"
    );
    assert!(!file.deleted, "file is deleted");
    file.file_type = edit.file_type;
    file.metadata_cid = edit.metadata_cid;
    file.content_cid = edit.content_cid;
    file.gate_cid = edit.gate_cid;
    file.version = edit.version;
    file.last_editor = edit.editor;
    Response::set_state(PortalState::File(file).encode())
}

/// At a file actor, from its portal or the app-file actor that found it: leaves a tombstone
/// (empty CIDs) so the file ids stay contiguous.
pub(crate) fn tombstone(input: &ReceiveInput, from: Actor, editor: AccountId) -> Response {
    let mut file = file_state(input);
    let a = addresses(input);
    assert!(
        file.portal == from.account_id
            || a.app_file(file.portal, &file.app_file_id) == from.account_id,
        "message must come from the file's portal"
    );
    assert!(!file.deleted, "file is already deleted");
    file.deleted = true;
    file.metadata_cid = String::new();
    file.content_cid = String::new();
    file.gate_cid = String::new();
    file.last_editor = editor;
    Response::set_state(PortalState::File(file).encode())
}

/// At a new app-file actor, from its portal.
pub(crate) fn bind_app_file(
    input: &ReceiveInput,
    from: Actor,
    app_file_id: &str,
    index: u64,
) -> Response {
    let portal = from.account_id;
    assert_receiver(
        input,
        addresses(input).app_file(portal, app_file_id),
        "app file",
    );
    assert!(input.is_authorized, "app file creation must be authorized");
    assert!(input.pre_state.is_empty(), "app file id is already in use");
    Response::set_state(PortalState::AppFile(AppFile { portal, index }).encode())
}

/// At an app-file actor, from its portal: tombstone the file it points to.
pub(crate) fn delete_by_app_file_id(
    input: &ReceiveInput,
    from: Actor,
    app_file_id: &str,
    editor: AccountId,
) -> Response {
    let portal = from.account_id;
    assert_receiver(
        input,
        addresses(input).app_file(portal, app_file_id),
        "app file",
    );
    let app_file = match state(input) {
        Some(PortalState::AppFile(app_file)) => app_file,
        _ => panic!("no file has this app file id"),
    };
    assert!(
        app_file.portal == portal,
        "app file belongs to another portal"
    );
    Response::keep_state()
        .send(send_to_pda(
            input,
            addresses(input).file(portal, app_file.index),
            file_seed(portal, app_file.index),
            &Internal::TombstoneFile { editor },
        ))
        .event(
            DeletedFile {
                portal,
                index: app_file.index,
                editor,
            }
            .to_program_event(),
        )
}
