use poise::serenity_prelude::{ChannelId, CreateEmbed, CreateMessage, Http, UserId};

use crate::config::Config;

/// Sends an embed to the notification channel and/or DMs the owner, whichever is configured.
pub async fn broadcast(http: &Http, cfg: &Config, embed: CreateEmbed) {
    if let Some(ch) = cfg.notify_channel_id {
        let msg = CreateMessage::new().embed(embed.clone());
        if let Err(e) = ChannelId::new(ch).send_message(http, msg).await {
            tracing::warn!("notify channel send failed: {e}");
        }
    }
    if let Some(uid) = cfg.owner_user_id {
        match UserId::new(uid).create_dm_channel(http).await {
            Ok(dm) => {
                if let Err(e) = dm
                    .send_message(http, CreateMessage::new().embed(embed))
                    .await
                {
                    tracing::warn!("owner DM failed: {e}");
                }
            }
            Err(e) => tracing::warn!("cannot open DM with owner: {e}"),
        }
    }
    if cfg.notify_channel_id.is_none() && cfg.owner_user_id.is_none() {
        tracing::warn!(
            "a notification was produced but NOTIFY_CHANNEL_ID / OWNER_USER_ID are not set"
        );
    }
}

/// Pastel pink used for the bot's embeds.
pub const PINK: u32 = 0xFFB7D5;
