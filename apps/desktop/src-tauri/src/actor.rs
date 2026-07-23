use std::path::PathBuf;

use p2p_core::AppService;
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, oneshot};

use crate::dto::{AttachmentDto, ChatEventDto, ContactDto, GroupDto, MessageDto};

/// Everything the frontend can ask the app node to do. `AppService` isn't
/// `Sync`/shareable-behind-a-plain-mutex-across-a-long-await in a way that
/// plays nicely with also needing to continuously poll it for network
/// events — so instead it's owned outright by one background task (`run`),
/// and commands reach it over this channel. This sidesteps a real deadlock
/// risk: a `Mutex<AppService>` held across `next_event().await` (which can
/// legitimately pend for a long time) would block every other command
/// until the next network event arrived.
pub enum Command {
    /// `display_name` is only used when there's no stored identity yet —
    /// see `AppService::load_or_create`. Responds with `(user_id,
    /// display_name)`; the display name comes back too so a *resume* (no
    /// name passed in) tells the caller what it actually resumed.
    Initialize {
        display_name: Option<String>,
        respond_to: oneshot::Sender<Result<(String, String), String>>,
    },
    Rename {
        new_display_name: String,
        respond_to: oneshot::Sender<Result<(), String>>,
    },
    AddContact {
        user_id: String,
        respond_to: oneshot::Sender<Result<(), String>>,
    },
    ListContacts {
        respond_to: oneshot::Sender<Result<Vec<ContactDto>, String>>,
    },
    SendDirectMessage {
        peer_user_id: String,
        body: String,
        attachment: Option<AttachmentDto>,
        respond_to: oneshot::Sender<Result<(), String>>,
    },
    ListMessages {
        conversation_id: String,
        respond_to: oneshot::Sender<Result<Vec<MessageDto>, String>>,
    },
    CreateGroup {
        name: String,
        respond_to: oneshot::Sender<Result<GroupDto, String>>,
    },
    InviteToGroup {
        group_id: String,
        member_user_id: String,
        respond_to: oneshot::Sender<Result<GroupDto, String>>,
    },
    CreateChannel {
        group_id: String,
        name: String,
        kind: p2p_core::ChannelKind,
        respond_to: oneshot::Sender<Result<GroupDto, String>>,
    },
    SendGroupMessage {
        group_id: String,
        channel_id: String,
        body: String,
        attachment: Option<AttachmentDto>,
        respond_to: oneshot::Sender<Result<(), String>>,
    },
    ListGroups {
        respond_to: oneshot::Sender<Result<Vec<GroupDto>, String>>,
    },
    JoinVoiceChannel {
        group_id: String,
        channel_id: String,
        respond_to: oneshot::Sender<Result<(), String>>,
    },
    LeaveVoiceChannel {
        respond_to: oneshot::Sender<Result<(), String>>,
    },
    SetVoiceChangerEnabled {
        enabled: bool,
        respond_to: oneshot::Sender<Result<(), String>>,
    },
    SetMicMuted {
        muted: bool,
        respond_to: oneshot::Sender<Result<(), String>>,
    },
    GetVoiceParticipants {
        respond_to: oneshot::Sender<Result<Vec<String>, String>>,
    },
    SetMicThresholdDb {
        db: f32,
        respond_to: oneshot::Sender<Result<(), String>>,
    },
    SetHearSelf {
        enabled: bool,
        respond_to: oneshot::Sender<Result<(), String>>,
    },
    GetVoiceSpeakingParticipants {
        respond_to: oneshot::Sender<Result<Vec<String>, String>>,
    },
}

#[derive(Clone)]
pub struct ActorHandle {
    tx: mpsc::Sender<Command>,
}

impl ActorHandle {
    pub fn spawn(app: AppHandle, data_dir: PathBuf, directory_url: String) -> Self {
        let (tx, rx) = mpsc::channel(32);
        tauri::async_runtime::spawn(run(app, data_dir, directory_url, rx));
        Self { tx }
    }

    async fn call<T>(
        &self,
        make_cmd: impl FnOnce(oneshot::Sender<T>) -> Command,
    ) -> Result<T, String> {
        let (respond_to, rx) = oneshot::channel();
        self.tx
            .send(make_cmd(respond_to))
            .await
            .map_err(|_| "app is shutting down".to_string())?;
        rx.await
            .map_err(|_| "app actor dropped the response channel".to_string())
    }

    /// `display_name` is `None` to resume an existing identity (its stored
    /// name is used and returned), `Some` only when creating a new one.
    pub async fn initialize(&self, display_name: Option<String>) -> Result<(String, String), String> {
        self.call(|respond_to| Command::Initialize {
            display_name,
            respond_to,
        })
        .await?
    }

    pub async fn rename(&self, new_display_name: String) -> Result<(), String> {
        self.call(|respond_to| Command::Rename {
            new_display_name,
            respond_to,
        })
        .await?
    }

    pub async fn add_contact(&self, user_id: String) -> Result<(), String> {
        self.call(|respond_to| Command::AddContact {
            user_id,
            respond_to,
        })
        .await?
    }

    pub async fn list_contacts(&self) -> Result<Vec<ContactDto>, String> {
        self.call(|respond_to| Command::ListContacts { respond_to })
            .await?
    }

    pub async fn send_direct_message(
        &self,
        peer_user_id: String,
        body: String,
        attachment: Option<AttachmentDto>,
    ) -> Result<(), String> {
        self.call(|respond_to| Command::SendDirectMessage {
            peer_user_id,
            body,
            attachment,
            respond_to,
        })
        .await?
    }

    pub async fn list_messages(&self, conversation_id: String) -> Result<Vec<MessageDto>, String> {
        self.call(|respond_to| Command::ListMessages {
            conversation_id,
            respond_to,
        })
        .await?
    }

    pub async fn create_group(&self, name: String) -> Result<GroupDto, String> {
        self.call(|respond_to| Command::CreateGroup { name, respond_to })
            .await?
    }

    pub async fn invite_to_group(
        &self,
        group_id: String,
        member_user_id: String,
    ) -> Result<GroupDto, String> {
        self.call(|respond_to| Command::InviteToGroup {
            group_id,
            member_user_id,
            respond_to,
        })
        .await?
    }

    pub async fn create_channel(
        &self,
        group_id: String,
        name: String,
        kind: p2p_core::ChannelKind,
    ) -> Result<GroupDto, String> {
        self.call(|respond_to| Command::CreateChannel {
            group_id,
            name,
            kind,
            respond_to,
        })
        .await?
    }

    pub async fn send_group_message(
        &self,
        group_id: String,
        channel_id: String,
        body: String,
        attachment: Option<AttachmentDto>,
    ) -> Result<(), String> {
        self.call(|respond_to| Command::SendGroupMessage {
            group_id,
            channel_id,
            body,
            attachment,
            respond_to,
        })
        .await?
    }

    pub async fn list_groups(&self) -> Result<Vec<GroupDto>, String> {
        self.call(|respond_to| Command::ListGroups { respond_to })
            .await?
    }

    pub async fn join_voice_channel(&self, group_id: String, channel_id: String) -> Result<(), String> {
        self.call(|respond_to| Command::JoinVoiceChannel {
            group_id,
            channel_id,
            respond_to,
        })
        .await?
    }

    pub async fn leave_voice_channel(&self) -> Result<(), String> {
        self.call(|respond_to| Command::LeaveVoiceChannel { respond_to })
            .await?
    }

    pub async fn set_voice_changer_enabled(&self, enabled: bool) -> Result<(), String> {
        self.call(|respond_to| Command::SetVoiceChangerEnabled { enabled, respond_to })
            .await?
    }

    pub async fn set_mic_muted(&self, muted: bool) -> Result<(), String> {
        self.call(|respond_to| Command::SetMicMuted { muted, respond_to })
            .await?
    }

    pub async fn get_voice_participants(&self) -> Result<Vec<String>, String> {
        self.call(|respond_to| Command::GetVoiceParticipants { respond_to })
            .await?
    }

    pub async fn set_mic_threshold_db(&self, db: f32) -> Result<(), String> {
        self.call(|respond_to| Command::SetMicThresholdDb { db, respond_to })
            .await?
    }

    pub async fn set_hear_self(&self, enabled: bool) -> Result<(), String> {
        self.call(|respond_to| Command::SetHearSelf { enabled, respond_to })
            .await?
    }

    pub async fn get_voice_speaking_participants(&self) -> Result<Vec<String>, String> {
        self.call(|respond_to| Command::GetVoiceSpeakingParticipants { respond_to })
            .await?
    }
}

async fn run(
    app: AppHandle,
    data_dir: PathBuf,
    directory_url: String,
    mut rx: mpsc::Receiver<Command>,
) {
    let mut service: Option<AppService> = None;
    // Cheap to tick often — `maybe_send_voice_heartbeat` only actually
    // re-announces once `voice::PRESENCE_HEARTBEAT` has elapsed (or
    // immediately retries after a failed send), and no-ops entirely
    // outside an active call.
    let mut voice_heartbeat = tokio::time::interval(std::time::Duration::from_secs(1));

    loop {
        match &mut service {
            None => {
                let Some(cmd) = rx.recv().await else { return };
                match cmd {
                    Command::Initialize {
                        display_name,
                        respond_to,
                    } => {
                        match AppService::load_or_create(
                            data_dir.clone(),
                            directory_url.clone(),
                            display_name,
                        )
                        .await
                        {
                            Ok(svc) => {
                                let user_id = svc.user_id();
                                let display_name = svc.display_name().to_string();
                                service = Some(svc);
                                let _ = respond_to.send(Ok((user_id, display_name)));
                            }
                            Err(e) => {
                                tracing::error!(error = %e, "failed to initialize app service");
                                let _ = respond_to.send(Err(e.to_string()));
                            }
                        }
                    }
                    other => reject_not_initialized(other),
                }
            }
            Some(_) => {
                tokio::select! {
                    maybe_cmd = rx.recv() => {
                        let Some(cmd) = maybe_cmd else { return };
                        handle_command(&app, service.as_mut().expect("service is Some in this branch"), cmd).await;
                    }
                    event = service.as_mut().expect("service is Some in this branch").next_event() => {
                        // `AppService::next_event` already auto-accepts group
                        // invites and self-heals unknown-sender contacts
                        // internally; this just tells the frontend to refetch
                        // the lists that might have just changed.
                        match &event {
                            p2p_core::ChatEvent::GroupKeyReceived { .. } => {
                                let _ = app.emit("groups-updated", ());
                            }
                            p2p_core::ChatEvent::DirectMessage { .. } => {
                                let _ = app.emit("contacts-updated", ());
                            }
                            _ => {}
                        }
                        if let Ok(dto) = ChatEventDto::try_from(event) {
                            let _ = app.emit("chat-event", dto);
                        }
                    }
                    _ = voice_heartbeat.tick() => {
                        service.as_mut().expect("service is Some in this branch").maybe_send_voice_heartbeat();
                    }
                }
            }
        }
    }
}

fn reject_not_initialized(cmd: Command) {
    let err = || "app is not initialized yet".to_string();
    match cmd {
        Command::Initialize { .. } => unreachable!(),
        Command::Rename { respond_to, .. } => {
            let _ = respond_to.send(Err(err()));
        }
        Command::AddContact { respond_to, .. } => {
            let _ = respond_to.send(Err(err()));
        }
        Command::ListContacts { respond_to } => {
            let _ = respond_to.send(Err(err()));
        }
        Command::SendDirectMessage { respond_to, .. } => {
            let _ = respond_to.send(Err(err()));
        }
        Command::ListMessages { respond_to, .. } => {
            let _ = respond_to.send(Err(err()));
        }
        Command::CreateGroup { respond_to, .. } => {
            let _ = respond_to.send(Err(err()));
        }
        Command::InviteToGroup { respond_to, .. } => {
            let _ = respond_to.send(Err(err()));
        }
        Command::CreateChannel { respond_to, .. } => {
            let _ = respond_to.send(Err(err()));
        }
        Command::SendGroupMessage { respond_to, .. } => {
            let _ = respond_to.send(Err(err()));
        }
        Command::ListGroups { respond_to } => {
            let _ = respond_to.send(Err(err()));
        }
        Command::JoinVoiceChannel { respond_to, .. } => {
            let _ = respond_to.send(Err(err()));
        }
        Command::LeaveVoiceChannel { respond_to } => {
            let _ = respond_to.send(Err(err()));
        }
        Command::SetVoiceChangerEnabled { respond_to, .. } => {
            let _ = respond_to.send(Err(err()));
        }
        Command::SetMicMuted { respond_to, .. } => {
            let _ = respond_to.send(Err(err()));
        }
        Command::GetVoiceParticipants { respond_to } => {
            let _ = respond_to.send(Err(err()));
        }
        Command::SetMicThresholdDb { respond_to, .. } => {
            let _ = respond_to.send(Err(err()));
        }
        Command::SetHearSelf { respond_to, .. } => {
            let _ = respond_to.send(Err(err()));
        }
        Command::GetVoiceSpeakingParticipants { respond_to } => {
            let _ = respond_to.send(Err(err()));
        }
    }
}

async fn handle_command(_app: &AppHandle, service: &mut AppService, cmd: Command) {
    match cmd {
        Command::Initialize { .. } => {
            unreachable!("handled by caller")
        }
        Command::Rename {
            new_display_name,
            respond_to,
        } => {
            let result = service
                .rename(new_display_name)
                .await
                .map_err(|e| e.to_string());
            let _ = respond_to.send(result);
        }
        Command::AddContact {
            user_id,
            respond_to,
        } => {
            let result = service
                .add_contact_by_user_id(&user_id)
                .await
                .map_err(|e| e.to_string());
            let _ = respond_to.send(result);
        }
        Command::ListContacts { respond_to } => {
            let result = service
                .list_contacts()
                .map(|v| v.into_iter().map(ContactDto::from).collect())
                .map_err(|e| e.to_string());
            let _ = respond_to.send(result);
        }
        Command::SendDirectMessage {
            peer_user_id,
            body,
            attachment,
            respond_to,
        } => {
            let result = match attachment.as_ref().map(AttachmentDto::to_payload).transpose() {
                Ok(attachment) => service
                    .send_direct_message(&peer_user_id, &body, attachment)
                    .await
                    .map_err(|e| e.to_string()),
                Err(e) => Err(e),
            };
            let _ = respond_to.send(result);
        }
        Command::ListMessages {
            conversation_id,
            respond_to,
        } => {
            let result = service
                .list_messages(&conversation_id)
                .map(|v| v.into_iter().map(MessageDto::from).collect())
                .map_err(|e| e.to_string());
            let _ = respond_to.send(result);
        }
        Command::CreateGroup { name, respond_to } => {
            let result = service
                .create_group(&name)
                .await
                .map(GroupDto::from)
                .map_err(|e| e.to_string());
            let _ = respond_to.send(result);
        }
        Command::InviteToGroup {
            group_id,
            member_user_id,
            respond_to,
        } => {
            let result = service
                .invite_to_group(&group_id, &member_user_id)
                .await
                .map(GroupDto::from)
                .map_err(|e| e.to_string());
            let _ = respond_to.send(result);
        }
        Command::CreateChannel {
            group_id,
            name,
            kind,
            respond_to,
        } => {
            let result = service
                .create_channel(&group_id, &name, kind)
                .await
                .map(GroupDto::from)
                .map_err(|e| e.to_string());
            let _ = respond_to.send(result);
        }
        Command::SendGroupMessage {
            group_id,
            channel_id,
            body,
            attachment,
            respond_to,
        } => {
            let result = match attachment.as_ref().map(AttachmentDto::to_payload).transpose() {
                Ok(attachment) => service
                    .send_group_message(&group_id, &channel_id, &body, attachment)
                    .await
                    .map_err(|e| e.to_string()),
                Err(e) => Err(e),
            };
            let _ = respond_to.send(result);
        }
        Command::ListGroups { respond_to } => {
            let result = service
                .list_groups()
                .map(|v| v.into_iter().map(GroupDto::from).collect())
                .map_err(|e| e.to_string());
            let _ = respond_to.send(result);
        }
        Command::JoinVoiceChannel {
            group_id,
            channel_id,
            respond_to,
        } => {
            let result = service
                .join_voice_channel(&group_id, &channel_id)
                .await
                .map_err(|e| e.to_string());
            let _ = respond_to.send(result);
        }
        Command::LeaveVoiceChannel { respond_to } => {
            let result = service.leave_voice_channel().map_err(|e| e.to_string());
            let _ = respond_to.send(result);
        }
        Command::SetVoiceChangerEnabled { enabled, respond_to } => {
            service.set_voice_changer_enabled(enabled);
            let _ = respond_to.send(Ok(()));
        }
        Command::SetMicMuted { muted, respond_to } => {
            service.set_mic_muted(muted);
            let _ = respond_to.send(Ok(()));
        }
        Command::GetVoiceParticipants { respond_to } => {
            let _ = respond_to.send(Ok(service.voice_participants()));
        }
        Command::SetMicThresholdDb { db, respond_to } => {
            service.set_mic_threshold_db(db);
            let _ = respond_to.send(Ok(()));
        }
        Command::SetHearSelf { enabled, respond_to } => {
            service.set_hear_self(enabled);
            let _ = respond_to.send(Ok(()));
        }
        Command::GetVoiceSpeakingParticipants { respond_to } => {
            let _ = respond_to.send(Ok(service.voice_speaking_participants()));
        }
    }
}
