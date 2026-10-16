//! Music, speaker and TV volume commands.

use crate::dispatch::DispatchCtx;
use crate::dispatch::targets::intent_room_to_canonical;
use crate::{music, response, tv};
use niles_core::RoomName;
use niles_speakers::SonosClient;
use niles_webos::VolumeChange;
use std::net::SocketAddr;
use std::sync::Arc;

/// "Stop the music", or a bare "stop" with no timer to stop: the music
/// here, or wherever it plays.
pub(crate) async fn stopped_music(
    ctx: &DispatchCtx,
    peer: SocketAddr,
    here: Option<&RoomName>,
) -> String {
    match ctx.music.stop_music(here).await {
        Ok(rooms) if rooms.is_empty() => {
            tracing::info!("[{peer}] no music to stop");
            response::no_music_playing()
        }
        Ok(rooms) => {
            tracing::info!("[{peer}] stopped the music in {}", rooms.join(", "));
            response::music_stopped(&rooms)
        }
        Err(e) => {
            tracing::warn!("[{peer}] stopping the music: {e}");
            response::music_failed(here.map_or("house", |r| r.as_str()))
        }
    }
}

/// The room a music command is for: the one named, or where it was said.
pub(crate) fn music_room(
    named: Option<&str>,
    origin: Option<&RoomName>,
) -> std::result::Result<RoomName, String> {
    match named {
        Some(raw) => intent_room_to_canonical(raw).map_err(|_| response::room_not_found(raw)),
        None => origin.cloned().ok_or_else(response::which_room_to_play),
    }
}

/// What to say once music was asked for. `None` when several stations
/// could be meant: the model sees them and picks, or asks.
pub(crate) fn music_reply(outcome: music::Outcome, room: &RoomName) -> Option<String> {
    Some(match outcome {
        music::Outcome::Playing(what) => response::playing(&what, room.as_str()),
        music::Outcome::Choose { .. } => return None,
        music::Outcome::NotFound(what) => response::music_not_found(&what),
        music::Outcome::WhichStation => response::which_station(),
        music::Outcome::NoSpeaker => response::no_speaker_in_room(room.as_str()),
        other => {
            tracing::warn!("[music] in {room}: {other:?}");
            response::music_failed(room.as_str())
        }
    })
}

/// "Set the kitchen volume to 30%": the room's Sonos, or the TV when
/// that is all the room has.
pub(crate) async fn room_volume_set(
    ctx: &DispatchCtx,
    peer: SocketAddr,
    room: &str,
    percent: u8,
) -> String {
    if let Some(said) = tv_room_volume(ctx, peer, room, VolumeChange::To(percent)).await {
        return said;
    }
    media_result_to_response(
        peer,
        room,
        media_dispatch(ctx, room, Reach::Every, move |c| async move {
            c.set_volume(percent).await
        })
        .await,
        response::media_volume(room, percent),
        "volume set",
    )
    .unwrap_or_default()
}

/// "Kitchen volume up": each of the room's Sonos by a step, or the TV
/// when that is all the room has.
pub(crate) async fn room_volume_step(
    ctx: &DispatchCtx,
    peer: SocketAddr,
    room: &str,
    delta: i16,
) -> String {
    let canonical = match intent_room_to_canonical(room) {
        Ok(r) => r,
        Err(reason) => {
            tracing::warn!("[{peer}] room {room:?} is not a valid registry name: {reason}");
            return response::room_not_found(room);
        }
    };
    let players = ctx.speakers.in_room(&canonical).await;
    if players.is_empty() {
        if let Some(said) = tv_room_volume(ctx, peer, room, VolumeChange::By(delta)).await {
            return said;
        }
        tracing::info!("[{peer}] no speaker in {canonical}");
        return response::no_speaker_in_room(canonical.as_str());
    }
    // Each by the same step from where it is: a speaker at the
    // back set quieter than the front stays quieter.
    let mut said = None;
    for player in &players {
        let current = match player.client.get_volume().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("[{peer}] speaker unreachable in {canonical}: {e}");
                return response::speaker_unreachable(canonical.as_str());
            }
        };
        let new = (current as i16 + delta).clamp(0, 100) as u8;
        if let Err(e) = player.client.set_volume(new).await {
            tracing::warn!("[{peer}] speaker unreachable in {canonical}: {e}");
            return response::speaker_unreachable(canonical.as_str());
        }
        println!(
            "[{peer}] media volume step on {} in {canonical}: {current} -> {new}",
            player.sonos.name
        );
        said.get_or_insert(new);
    }
    response::media_volume(canonical.as_str(), said.unwrap_or_default())
}

/// The room's volume on its TV, when the TV stands there and the room
/// has no Sonos. `None` leaves it to the speakers, or to "no speaker".
pub(crate) async fn tv_room_volume(
    ctx: &DispatchCtx,
    peer: SocketAddr,
    room: &str,
    change: VolumeChange,
) -> Option<String> {
    let canonical = intent_room_to_canonical(room).ok()?;
    let has_speakers = !ctx.speakers.in_room(&canonical).await.is_empty();
    let result = ctx.tv.room_volume(&canonical, has_speakers, change).await?;
    tracing::info!("[{peer}] {change:?} in {canonical} on the TV: {result:?}");
    Some(match result {
        Ok(after) => tv::said(change, after),
        Err(say) => say,
    })
}

/// "Turn the TV up": the TV's sound, which is the Sonos in its room
/// when it has one — the user's own soundbar — and otherwise the TV's
/// own speakers.
pub(crate) async fn tv_volume(ctx: &DispatchCtx, peer: SocketAddr, change: VolumeChange) -> String {
    if let Some(room) = ctx.tv.room()
        && !ctx.speakers.in_room(&room).await.is_empty()
    {
        match change {
            VolumeChange::To(percent) => {
                return room_volume_set(ctx, peer, room.as_str(), percent).await;
            }
            VolumeChange::By(delta) => {
                return room_volume_step(ctx, peer, room.as_str(), delta).await;
            }
            VolumeChange::Mute(_) => {}
        }
    }
    ctx.tv.volume(change).await.unwrap_or_else(|e| e)
}

#[derive(Debug)]
pub(crate) enum MediaDispatchError {
    BadRoom(String),
    NoSpeaker(RoomName),
    Unreachable(RoomName, String),
}

/// Which of a room's speakers an action goes to.
#[derive(Clone, Copy)]
pub(crate) enum Reach {
    /// The leader of each group the room's speakers are in: play,
    /// pause and skip act on a group, and a member refuses them.
    Leaders,
    /// Each of them: volume is per speaker.
    Every,
}

pub(crate) async fn media_dispatch<F, Fut>(
    ctx: &DispatchCtx,
    room_raw: &str,
    reach: Reach,
    op: F,
) -> std::result::Result<(), MediaDispatchError>
where
    F: Fn(Arc<SonosClient>) -> Fut,
    Fut: std::future::Future<Output = niles_speakers::Result<()>>,
{
    let canonical = intent_room_to_canonical(room_raw).map_err(MediaDispatchError::BadRoom)?;
    let targets: Vec<Arc<SonosClient>> = match reach {
        Reach::Leaders => ctx.speakers.leaders(&canonical).await,
        Reach::Every => ctx
            .speakers
            .in_room(&canonical)
            .await
            .into_iter()
            .map(|player| player.client)
            .collect(),
    };
    if targets.is_empty() {
        return Err(MediaDispatchError::NoSpeaker(canonical));
    }
    for client in targets {
        op(client)
            .await
            .map_err(|e| MediaDispatchError::Unreachable(canonical.clone(), e.to_string()))?;
    }
    Ok(())
}

/// Turns the result of a `media_dispatch` call into the spoken
/// response the user hears, logging warnings for error cases.
pub(crate) fn media_result_to_response(
    peer: SocketAddr,
    room: &str,
    result: std::result::Result<(), MediaDispatchError>,
    ok_msg: String,
    action: &str,
) -> Option<String> {
    match result {
        Ok(()) => {
            tracing::info!("[{peer}] media {action} in {room}");
            Some(ok_msg)
        }
        Err(MediaDispatchError::BadRoom(reason)) => {
            tracing::warn!("[{peer}] room {room:?} is not a valid registry name: {reason}");
            Some(response::room_not_found(room))
        }
        Err(MediaDispatchError::NoSpeaker(r)) => {
            tracing::info!("[{peer}] no speaker in {r}");
            Some(response::no_speaker_in_room(r.as_str()))
        }
        Err(MediaDispatchError::Unreachable(r, e)) => {
            tracing::warn!("[{peer}] speaker unreachable in {r}: {e}");
            Some(response::speaker_unreachable(r.as_str()))
        }
    }
}
