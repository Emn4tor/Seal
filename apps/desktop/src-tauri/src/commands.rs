use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use img_parts::ImageEXIF;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::account_manager::AccountManager;
use crate::accounts;
use crate::actor::ActorHandle;
use crate::dto::{
    AccountSummaryDto, AccountsStateDto, AttachmentDto, BootDecisionDto, ContactDto, ExifFieldDto,
    GroupDto, MessageDto,
};
use crate::{embedded_directory, server_config, AppPaths};

/// The compiled-in "Seal" network, if this build has one configured —
/// `None` means no official server is set up yet (see the README for how
/// to bake one in). Distinct from the local test server, which is always
/// available regardless.
#[tauri::command]
pub fn get_official_server_url() -> Option<String> {
    server_config::official_server_url()
}

/// The server the user chose on a previous run, if this is a returning
/// user — lets the frontend skip asking again.
#[tauri::command]
pub fn get_saved_server_url(paths: State<'_, AppPaths>) -> Option<String> {
    server_config::load_saved(&paths.shared_data_dir)
}

/// Persists a server choice for next launch without starting anything —
/// used by Settings' "change server" flow, which takes effect on restart
/// rather than trying to hot-swap a running P2P node's backend connection.
#[tauri::command]
pub fn save_server_url_for_next_launch(
    paths: State<'_, AppPaths>,
    server_url: String,
) -> Result<(), String> {
    server_config::save(&paths.shared_data_dir, &server_url).map_err(|e| e.to_string())
}

/// Resolves and starts the chosen backend (embedded sentinel or a real
/// URL). Doesn't touch any account — which one to load isn't decided until
/// `resolve_boot_account` / `create_account` / `resume_account`, since more
/// than one can now exist on this device.
#[tauri::command]
pub async fn start_backend(
    paths: State<'_, AppPaths>,
    server_url: String,
) -> Result<String, String> {
    server_config::save(&paths.shared_data_dir, &server_url).map_err(|e| e.to_string())?;

    let resolved_url = if server_url == server_config::EMBEDDED_SENTINEL {
        embedded_directory::ensure_running(paths.shared_data_dir.clone()).await
    } else {
        server_url
    };
    Ok(resolved_url)
}

/// The one call the frontend needs to decide what to show at startup — see
/// `accounts::BootDecision` for what each outcome means.
#[tauri::command]
pub fn resolve_boot_account(paths: State<'_, AppPaths>) -> BootDecisionDto {
    accounts::resolve_boot(&paths.shared_data_dir).into()
}

/// Every account on this device, for the account picker / Settings'
/// "Accounts on this device" list.
#[tauri::command]
pub fn list_accounts(paths: State<'_, AppPaths>) -> AccountsStateDto {
    let file = accounts::load(&paths.shared_data_dir);
    AccountsStateDto {
        accounts: file
            .accounts
            .into_iter()
            .map(AccountSummaryDto::from)
            .collect(),
        active_account_id: file.active_account_id,
    }
}

/// Creates a brand-new identity, loads it, and makes it the active account.
#[tauri::command]
pub async fn create_account(
    app: AppHandle,
    paths: State<'_, AppPaths>,
    manager: State<'_, AccountManager>,
    server_url: String,
    display_name: String,
) -> Result<AccountSummaryDto, String> {
    let account_id = accounts::new_account_id();
    let dir = accounts::account_dir(&paths.shared_data_dir, &account_id);

    let handle = ActorHandle::spawn(app, dir, server_url);
    let (user_id, display_name) = handle.initialize(Some(display_name)).await?;
    manager.set_current(handle).await;

    let entry = accounts::AccountEntry {
        account_id: account_id.clone(),
        user_id,
        display_name,
        created_at: accounts::now(),
    };
    let mut file = accounts::load(&paths.shared_data_dir);
    file.accounts.push(entry.clone());
    file.active_account_id = Some(account_id);
    accounts::save(&paths.shared_data_dir, &file).map_err(|e| e.to_string())?;

    Ok(entry.into())
}

/// Loads an existing identity by account id (no display name involved —
/// its stored one is used) and makes it the active account. Used both for
/// auto-login at boot and for an explicit "switch account" from Settings.
#[tauri::command]
pub async fn resume_account(
    app: AppHandle,
    paths: State<'_, AppPaths>,
    manager: State<'_, AccountManager>,
    server_url: String,
    account_id: String,
) -> Result<AccountSummaryDto, String> {
    let mut file = accounts::load(&paths.shared_data_dir);
    let created_at = file
        .accounts
        .iter()
        .find(|a| a.account_id == account_id)
        .map(|a| a.created_at)
        .ok_or_else(|| "no such account on this device".to_string())?;

    let dir = accounts::account_dir(&paths.shared_data_dir, &account_id);
    let handle = ActorHandle::spawn(app, dir, server_url);
    let (user_id, display_name) = handle.initialize(None).await?;
    manager.set_current(handle).await;

    // Rebuild the cached entry from what was actually loaded rather than
    // trusting the old cached copy — keeps `accounts.json` honest even if
    // it ever drifted from the account's own local storage.
    let entry = accounts::AccountEntry {
        account_id: account_id.clone(),
        user_id,
        display_name,
        created_at,
    };
    file.accounts.retain(|a| a.account_id != account_id);
    file.accounts.push(entry.clone());
    file.active_account_id = Some(account_id);
    accounts::save(&paths.shared_data_dir, &file).map_err(|e| e.to_string())?;

    Ok(entry.into())
}

/// Renames the currently active account: re-registers with the directory
/// and re-saves locally (see `p2p_core::AppService::rename`), then updates
/// the cached copy in `accounts.json` so the picker/list reflects it
/// without needing to reload the account.
#[tauri::command]
pub async fn rename_account(
    paths: State<'_, AppPaths>,
    manager: State<'_, AccountManager>,
    new_display_name: String,
) -> Result<(), String> {
    manager
        .current()
        .await?
        .rename(new_display_name.clone())
        .await?;

    let mut file = accounts::load(&paths.shared_data_dir);
    if let Some(active_id) = file.active_account_id.clone() {
        if let Some(entry) = file.accounts.iter_mut().find(|a| a.account_id == active_id) {
            entry.display_name = new_display_name;
        }
    }
    accounts::save(&paths.shared_data_dir, &file).map_err(|e| e.to_string())?;
    Ok(())
}

/// Removes one account from this device — crypto-shreds its keychain entry
/// and deletes its local database (`storage::panic_purge`, unchanged),
/// same as the "delete everything" path but scoped to just this identity.
/// Works on the active account too (tears its live handle down first).
#[tauri::command]
pub async fn remove_account(
    paths: State<'_, AppPaths>,
    manager: State<'_, AccountManager>,
    account_id: String,
) -> Result<(), String> {
    let mut file = accounts::load(&paths.shared_data_dir);
    let is_active = file.active_account_id.as_deref() == Some(account_id.as_str());
    if is_active {
        manager.clear_current().await;
    }

    let dir = accounts::account_dir(&paths.shared_data_dir, &account_id);
    let keychain = identity::Keychain::for_app_data_dir(&dir).map_err(|e| e.to_string())?;
    storage::panic_purge(&dir.join("local.sqlite3"), &keychain).map_err(|e| e.to_string())?;

    file.accounts.retain(|a| a.account_id != account_id);
    if is_active {
        file.active_account_id = None;
    }
    accounts::save(&paths.shared_data_dir, &file).map_err(|e| e.to_string())?;
    Ok(())
}

/// The local "panic purge": wipes *every* account on this device, not just
/// the active one — each gets the same crypto-shred `remove_account` uses.
/// Has no effect on the directory server or on anyone else's copy of Seal.
#[tauri::command]
pub async fn panic_purge(
    paths: State<'_, AppPaths>,
    manager: State<'_, AccountManager>,
) -> Result<(), String> {
    manager.clear_current().await;

    let file = accounts::load(&paths.shared_data_dir);
    for account in &file.accounts {
        let dir = accounts::account_dir(&paths.shared_data_dir, &account.account_id);
        let keychain = identity::Keychain::for_app_data_dir(&dir).map_err(|e| e.to_string())?;
        storage::panic_purge(&dir.join("local.sqlite3"), &keychain).map_err(|e| e.to_string())?;
    }
    accounts::save(&paths.shared_data_dir, &accounts::AccountsFile::default())
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn add_contact(state: State<'_, AccountManager>, user_id: String) -> Result<(), String> {
    state.current().await?.add_contact(user_id).await
}

#[tauri::command]
pub async fn list_contacts(state: State<'_, AccountManager>) -> Result<Vec<ContactDto>, String> {
    state.current().await?.list_contacts().await
}

#[tauri::command]
pub async fn send_direct_message(
    state: State<'_, AccountManager>,
    peer_user_id: String,
    body: String,
    attachment: Option<AttachmentDto>,
) -> Result<(), String> {
    state
        .current()
        .await?
        .send_direct_message(peer_user_id, body, attachment)
        .await
}

#[tauri::command]
pub async fn list_messages(
    state: State<'_, AccountManager>,
    conversation_id: String,
) -> Result<Vec<MessageDto>, String> {
    state.current().await?.list_messages(conversation_id).await
}

#[tauri::command]
pub async fn create_group(
    state: State<'_, AccountManager>,
    name: String,
) -> Result<GroupDto, String> {
    state.current().await?.create_group(name).await
}

#[tauri::command]
pub async fn invite_to_group(
    state: State<'_, AccountManager>,
    group_id: String,
    member_user_id: String,
) -> Result<GroupDto, String> {
    state
        .current()
        .await?
        .invite_to_group(group_id, member_user_id)
        .await
}

/// Owner-only (enforced server-side); returns the whole updated group so
/// the frontend just refreshes rather than needing a separate channel-list
/// fetch.
#[tauri::command]
pub async fn create_channel(
    state: State<'_, AccountManager>,
    group_id: String,
    name: String,
    kind: p2p_core::ChannelKind,
) -> Result<GroupDto, String> {
    state
        .current()
        .await?
        .create_channel(group_id, name, kind)
        .await
}

#[tauri::command]
pub async fn send_group_message(
    state: State<'_, AccountManager>,
    group_id: String,
    channel_id: String,
    body: String,
    attachment: Option<AttachmentDto>,
) -> Result<(), String> {
    state
        .current()
        .await?
        .send_group_message(group_id, channel_id, body, attachment)
        .await
}

#[tauri::command]
pub async fn list_groups(state: State<'_, AccountManager>) -> Result<Vec<GroupDto>, String> {
    state.current().await?.list_groups().await
}

#[tauri::command]
pub async fn join_voice_channel(
    state: State<'_, AccountManager>,
    group_id: String,
    channel_id: String,
) -> Result<(), String> {
    state
        .current()
        .await?
        .join_voice_channel(group_id, channel_id)
        .await
}

#[tauri::command]
pub async fn leave_voice_channel(state: State<'_, AccountManager>) -> Result<(), String> {
    state.current().await?.leave_voice_channel().await
}

#[tauri::command]
pub async fn set_voice_changer_enabled(
    state: State<'_, AccountManager>,
    enabled: bool,
) -> Result<(), String> {
    state.current().await?.set_voice_changer_enabled(enabled).await
}

#[tauri::command]
pub async fn set_mic_muted(state: State<'_, AccountManager>, muted: bool) -> Result<(), String> {
    state.current().await?.set_mic_muted(muted).await
}

#[tauri::command]
pub async fn get_voice_participants(state: State<'_, AccountManager>) -> Result<Vec<String>, String> {
    state.current().await?.get_voice_participants().await
}

#[tauri::command]
pub async fn set_mic_threshold_db(state: State<'_, AccountManager>, db: f32) -> Result<(), String> {
    state.current().await?.set_mic_threshold_db(db).await
}

#[tauri::command]
pub async fn set_hear_self(state: State<'_, AccountManager>, enabled: bool) -> Result<(), String> {
    state.current().await?.set_hear_self(enabled).await
}

#[tauri::command]
pub async fn get_voice_speaking_participants(state: State<'_, AccountManager>) -> Result<Vec<String>, String> {
    state.current().await?.get_voice_speaking_participants().await
}

/// Removes EXIF metadata from image bytes losslessly (segment-level, no
/// recompression) if the container format is one `img-parts` recognizes.
/// Returns the bytes unchanged with `false` if not — reported honestly by
/// the caller rather than claiming a strip that didn't happen, since this
/// is a privacy-relevant guarantee, not just a cosmetic one.
fn strip_exif_from_image(data: Vec<u8>) -> (Vec<u8>, bool) {
    match img_parts::DynImage::from_bytes(bytes::Bytes::from(data.clone())) {
        Ok(Some(mut img)) => {
            img.set_exif(None);
            (img.encoder().bytes().to_vec(), true)
        }
        _ => (data, false),
    }
}

/// Opens a native "choose a file" dialog, reads the file, guesses its MIME
/// type from the extension, and — if it's an image and `strip_exif` is
/// set — removes its EXIF metadata losslessly (segment-level, no
/// recompression) before it ever enters the send pipeline. `Ok(None)` if
/// the user cancels. Rejects anything over the app-level size cap
/// (`p2p_core::MAX_ATTACHMENT_SIZE`) before it goes any further.
#[tauri::command]
pub async fn pick_attachment(
    app: AppHandle,
    strip_exif: bool,
) -> Result<Option<AttachmentDto>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_file(move |path| {
        let _ = tx.send(path);
    });
    let Some(file_path) = rx
        .await
        .map_err(|_| "file dialog closed unexpectedly".to_string())?
    else {
        return Ok(None);
    };
    let path = file_path
        .into_path()
        .map_err(|e| format!("couldn't resolve the picked file's path: {e}"))?;
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());

    let mut data = std::fs::read(&path).map_err(|e| format!("couldn't read the file: {e}"))?;
    if data.len() > p2p_core::MAX_ATTACHMENT_SIZE {
        return Err(format!(
            "\"{filename}\" is too large ({} MB, max {} MB)",
            data.len() / (1024 * 1024),
            p2p_core::MAX_ATTACHMENT_SIZE / (1024 * 1024)
        ));
    }

    let mime_type = mime_guess::from_path(&path)
        .first_or_octet_stream()
        .essence_str()
        .to_string();

    let mut exif_stripped = false;
    if strip_exif && mime_type.starts_with("image/") {
        let (stripped, did_strip) = strip_exif_from_image(data);
        data = stripped;
        exif_stripped = did_strip;
    }

    Ok(Some(AttachmentDto {
        filename,
        mime_type,
        size: data.len() as u64,
        exif_stripped,
        data_base64: STANDARD.encode(&data),
    }))
}

/// Parses EXIF tags out of image bytes already in the frontend's hands (a
/// received message's attachment) — no disk I/O, just decode + parse. An
/// empty result (rather than an error) means no EXIF data was found, which
/// is the common, unremarkable case, not a failure.
#[tauri::command]
pub fn get_image_exif(data_base64: String) -> Result<Vec<ExifFieldDto>, String> {
    let data = STANDARD
        .decode(&data_base64)
        .map_err(|e| format!("invalid image data: {e}"))?;
    let mut cursor = std::io::Cursor::new(&data);
    let exif = match exif::Reader::new().read_from_container(&mut cursor) {
        Ok(exif) => exif,
        Err(_) => return Ok(Vec::new()),
    };
    Ok(exif
        .fields()
        .map(|f| ExifFieldDto {
            tag: f.tag.to_string(),
            value: f.display_value().with_unit(&exif).to_string(),
        })
        .collect())
}

/// Opens a native "save as" dialog and writes the decoded bytes there.
/// Returns `false` (not an error) if the user cancels.
#[tauri::command]
pub async fn save_attachment(
    app: AppHandle,
    data_base64: String,
    suggested_filename: String,
) -> Result<bool, String> {
    let data = STANDARD
        .decode(&data_base64)
        .map_err(|e| format!("invalid attachment data: {e}"))?;

    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(&suggested_filename)
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    let Some(file_path) = rx
        .await
        .map_err(|_| "save dialog closed unexpectedly".to_string())?
    else {
        return Ok(false);
    };
    let path = file_path
        .into_path()
        .map_err(|e| format!("couldn't resolve the save location: {e}"))?;
    std::fs::write(&path, &data).map_err(|e| format!("couldn't write the file: {e}"))?;
    Ok(true)
}

#[cfg(test)]
mod attachment_tests {
    use img_parts::ImageEXIF;

    use super::strip_exif_from_image;

    /// A real, freshly-encoded 2x2 JPEG (via the `image` crate) with a
    /// minimal fake EXIF segment injected via `img-parts` — not a hand-rolled
    /// byte string, so this exercises the same container format the app
    /// actually sees.
    fn tiny_jpeg_with_exif() -> Vec<u8> {
        let img = image::RgbImage::from_pixel(2, 2, image::Rgb([255, 0, 0]));
        let mut jpeg_bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut jpeg_bytes),
            image::ImageFormat::Jpeg,
        )
        .expect("encode test fixture jpeg");

        let mut jpeg = img_parts::jpeg::Jpeg::from_bytes(bytes::Bytes::from(jpeg_bytes))
            .expect("parse the freshly-encoded test jpeg");
        jpeg.set_exif(Some(bytes::Bytes::from_static(
            b"Exif\0\0MM\0*\0\0\0\x08\0\0",
        )));
        jpeg.encoder().bytes().to_vec()
    }

    #[test]
    fn strip_removes_metadata_but_keeps_a_valid_image() {
        let with_exif = tiny_jpeg_with_exif();
        let parsed = img_parts::DynImage::from_bytes(bytes::Bytes::from(with_exif.clone()))
            .expect("parses as a known container")
            .expect("recognized as jpeg");
        assert!(
            parsed.exif().is_some(),
            "fixture should actually have exif before stripping"
        );

        let (stripped, did_strip) = strip_exif_from_image(with_exif);
        assert!(did_strip);

        let parsed_after = img_parts::DynImage::from_bytes(bytes::Bytes::from(stripped.clone()))
            .expect("stripped bytes still parse as a known container")
            .expect("still recognized as jpeg");
        assert!(parsed_after.exif().is_none(), "exif should be gone");

        // Not just "some bytes that happen to lack an exif tag" — still a
        // genuinely valid, decodable image afterward.
        image::load_from_memory(&stripped).expect("stripped image still decodes");
    }

    #[test]
    fn strip_on_non_image_bytes_reports_honestly() {
        let (unchanged, did_strip) = strip_exif_from_image(b"not an image at all".to_vec());
        assert!(!did_strip);
        assert_eq!(unchanged, b"not an image at all");
    }
}
