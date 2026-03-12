use std::{path::PathBuf, sync::Arc};

use anyhow::Result;
use tokio::sync::{mpsc, Mutex as TokioMutex};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use wacore::{proto_helpers::MessageExt, types::events::Event};
use whatsapp_rust::bot::Bot;

use super::{attachment, SidecarEvent, SidecarMessage, Source};

pub(crate) struct WhatsAppManager {
    client: Arc<TokioMutex<Option<Arc<whatsapp_rust::Client>>>>,
    cancel: CancellationToken,
}

impl WhatsAppManager {
    pub(crate) async fn start(
        auth_dir: &str,
        assistant_name: &str,
        data_dir: PathBuf,
        event_tx: mpsc::UnboundedSender<SidecarEvent>,
        cancel: CancellationToken,
    ) -> Result<Arc<Self>> {
        let db_path = format!("{}/whatsapp.db", auth_dir);
        let backend = Arc::new(whatsapp_rust_sqlite_storage::SqliteStore::new(&db_path).await?);
        let transport = whatsapp_rust_tokio_transport::TokioWebSocketTransportFactory::new();
        let http_client = whatsapp_rust_ureq_http_client::UreqHttpClient::new();

        let assistant_name_lower = assistant_name.to_lowercase();
        let data_dir2 = data_dir.clone();
        let event_tx2 = event_tx.clone();
        let client_slot: Arc<TokioMutex<Option<Arc<whatsapp_rust::Client>>>> =
            Arc::new(TokioMutex::new(None));
        let client_slot2 = Arc::clone(&client_slot);

        let mut bot = Bot::builder()
            .with_backend(backend)
            .with_transport_factory(transport)
            .with_http_client(http_client)
            .on_event(move |event, client| {
                let event_tx = event_tx2.clone();
                let assistant_name = assistant_name_lower.clone();
                let data_dir = data_dir2.clone();
                let client_slot = Arc::clone(&client_slot2);
                async move {
                    handle_event(
                        event,
                        client,
                        &event_tx,
                        &assistant_name,
                        &data_dir,
                        &client_slot,
                    )
                    .await;
                }
            })
            .build()
            .await?;

        let wa_client = bot.client();
        *client_slot.lock().await = Some(Arc::clone(&wa_client));

        let manager = Arc::new(Self {
            client: Arc::clone(&client_slot),
            cancel: cancel.clone(),
        });

        let cancel2 = cancel.clone();
        tokio::spawn(async move {
            tokio::select! {
                result = async {
                    let handle = bot.run().await?;
                    handle.await.map_err(|e| anyhow::anyhow!("bot task panicked: {e}"))?;
                    Ok::<(), anyhow::Error>(())
                } => {
                    if let Err(e) = result {
                        warn!("WhatsApp bot exited: {e}");
                    }
                }
                _ = cancel2.cancelled() => {
                    let lock = client_slot.lock().await;
                    if let Some(ref c) = *lock {
                        c.disconnect().await;
                    }
                }
            }
        });

        Ok(manager)
    }

    pub(crate) async fn send(&self, jid_str: &str, text: &str, _quote_id: Option<&str>) {
        let lock = self.client.lock().await;
        let Some(ref client) = *lock else { return };
        let jid: wacore_binary::jid::Jid = match jid_str.parse() {
            Ok(j) => j,
            Err(e) => {
                warn!("Invalid WhatsApp JID '{jid_str}': {e}");
                return;
            },
        };
        let message = waproto::whatsapp::Message {
            conversation: Some(text.to_string()),
            ..Default::default()
        };
        if let Err(e) = client.send_message(jid, message).await {
            warn!("WhatsApp send error: {e}");
        }
    }

    pub(crate) async fn send_typing(&self, jid_str: &str) {
        let lock = self.client.lock().await;
        let Some(ref client) = *lock else { return };
        let jid: wacore_binary::jid::Jid = match jid_str.parse() {
            Ok(j) => j,
            Err(e) => {
                warn!("Invalid WhatsApp JID '{jid_str}': {e}");
                return;
            },
        };
        let _ = client.chatstate().send_composing(&jid).await;
    }

    pub(crate) async fn logout(&self) {
        let lock = self.client.lock().await;
        if let Some(ref client) = *lock {
            client.disconnect().await;
        }
    }

    pub(crate) async fn shutdown(&self) {
        self.cancel.cancel();
    }
}

async fn handle_event(
    event: Event,
    client: Arc<whatsapp_rust::Client>,
    event_tx: &mpsc::UnboundedSender<SidecarEvent>,
    assistant_name: &str,
    data_dir: &PathBuf,
    client_slot: &TokioMutex<Option<Arc<whatsapp_rust::Client>>>,
) {
    match event {
        Event::Connected(_) => {
            let pm = client.persistence_manager();
            let snapshot = pm.get_device_snapshot().await;
            let jid = snapshot
                .pn
                .as_ref()
                .map(|j| j.to_string())
                .unwrap_or_default();
            info!("WhatsApp connected as {jid}");
            *client_slot.lock().await = Some(Arc::clone(&client));
            let _ = event_tx.send(SidecarEvent::WaConnected { jid });
        },
        Event::PairingQrCode { code, .. } => {
            info!("WhatsApp QR code generated");
            let _ = event_tx.send(SidecarEvent::WaQr { data: code });
        },
        Event::LoggedOut(logout_info) => {
            let _ = event_tx.send(SidecarEvent::Disconnected {
                source: Source::WhatsApp,
                reason: format!("logged out: {:?}", logout_info.reason),
            });
        },
        Event::Disconnected(_) => {
            let _ = event_tx.send(SidecarEvent::Disconnected {
                source: Source::WhatsApp,
                reason: "disconnected".to_string(),
            });
        },
        Event::Message(msg, msg_info) => {
            handle_message(*msg, msg_info, &client, event_tx, assistant_name, data_dir).await;
        },
        _ => {},
    }
}

async fn handle_message(
    msg: waproto::whatsapp::Message,
    info: wacore::types::message::MessageInfo,
    client: &whatsapp_rust::Client,
    event_tx: &mpsc::UnboundedSender<SidecarEvent>,
    assistant_name: &str,
    data_dir: &PathBuf,
) {
    if info.source.is_from_me {
        return;
    }

    let chat_jid = info.source.chat.to_string();
    let sender_jid = info.source.sender.to_string();
    let is_group = chat_jid.ends_with("@g.us");

    let sender_name = if info.push_name.is_empty() {
        sender_jid.split('@').next().unwrap_or("").to_string()
    } else {
        info.push_name.clone()
    };

    let text = msg.text_content().unwrap_or_default().to_string();

    let mut attachments = Vec::new();

    if let Some(ref img) = msg.image_message {
        if let Ok(bytes) = client.download(img.as_ref()).await {
            let mimetype = img.mimetype.as_deref().unwrap_or("image/jpeg");
            let filename = format!("image.{}", mime_ext(mimetype));
            if let Ok(sa) =
                attachment::save_bytes(&bytes, "whatsapp", &filename, mimetype, data_dir).await
            {
                attachments.push(sa);
            }
        }
    }

    if let Some(ref doc) = msg.document_message {
        if let Ok(bytes) = client.download(doc.as_ref()).await {
            let mimetype = doc
                .mimetype
                .as_deref()
                .unwrap_or("application/octet-stream");
            let filename = doc.file_name.as_deref().unwrap_or("document").to_string();
            if let Ok(sa) =
                attachment::save_bytes(&bytes, "whatsapp", &filename, mimetype, data_dir).await
            {
                attachments.push(sa);
            }
        }
    }

    if let Some(ref vid) = msg.video_message {
        if let Ok(bytes) = client.download(vid.as_ref()).await {
            let mimetype = vid.mimetype.as_deref().unwrap_or("video/mp4");
            let filename = format!("video.{}", mime_ext(mimetype));
            if let Ok(sa) =
                attachment::save_bytes(&bytes, "whatsapp", &filename, mimetype, data_dir).await
            {
                attachments.push(sa);
            }
        }
    }

    if let Some(ref aud) = msg.audio_message {
        if let Ok(bytes) = client.download(aud.as_ref()).await {
            let mimetype = aud.mimetype.as_deref().unwrap_or("audio/ogg");
            let filename = format!("audio.{}", mime_ext(mimetype));
            if let Ok(sa) =
                attachment::save_bytes(&bytes, "whatsapp", &filename, mimetype, data_dir).await
            {
                attachments.push(sa);
            }
        }
    }

    if text.is_empty() && attachments.is_empty() {
        return;
    }

    let pm = client.persistence_manager();
    let self_jid = pm
        .get_device_snapshot()
        .await
        .pn
        .as_ref()
        .map(|j| j.to_string())
        .unwrap_or_default();

    let mentions_by_name = text.to_lowercase().contains(&format!("@{assistant_name}"));
    let mentions_by_jid = if let Some(ref ext) = msg.extended_text_message {
        if let Some(ref ctx) = ext.context_info {
            ctx.mentioned_jid
                .iter()
                .any(|j| !self_jid.is_empty() && j.split('@').next() == self_jid.split('@').next())
        } else {
            false
        }
    } else {
        false
    };

    let timestamp = info.timestamp.timestamp();

    let _ = event_tx.send(SidecarEvent::Message(SidecarMessage {
        source: Source::WhatsApp,
        id: info.id.clone(),
        chat_id: chat_jid,
        sender: sender_jid,
        sender_name,
        text,
        attachments,
        timestamp,
        is_group,
        mentions_bot: mentions_by_jid || mentions_by_name,
        user_id: None,
    }));
}

fn mime_ext(mimetype: &str) -> &str {
    match mimetype {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "video/mp4" => "mp4",
        "audio/ogg" => "ogg",
        "audio/mpeg" => "mp3",
        _ => "bin",
    }
}
