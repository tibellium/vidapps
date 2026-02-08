use crate::engine::{ChannelFilter, ProcessPhase, Transform};

use super::types::Channel;

/// Apply the full process phase (filter + transforms) to a list of channels.
pub fn apply_process_phase(channels: Vec<Channel>, process: &ProcessPhase) -> Vec<Channel> {
    let mut channels = apply_filter(channels, process.filter.as_ref());

    for transform in &process.transforms {
        apply_transform(&mut channels, transform);
    }

    channels
}

/// Filter channels based on name and/or id lists.
fn apply_filter(channels: Vec<Channel>, filter: Option<&ChannelFilter>) -> Vec<Channel> {
    let Some(filter) = filter else {
        return channels;
    };

    let filtered: Vec<_> = channels
        .into_iter()
        .filter(|c| {
            let name_match = filter.name.is_empty()
                || c.name
                    .as_ref()
                    .map(|n| filter.name.contains(n))
                    .unwrap_or(false);

            let id_match = filter.id.is_empty() || filter.id.contains(&c.id);

            name_match && id_match
        })
        .collect();

    println!(
        "[process] Filter applied: {} channels remaining",
        filtered.len()
    );

    filtered
}

/// Apply a single transform to a list of channels.
fn apply_transform(channels: &mut [Channel], transform: &Transform) {
    match transform {
        Transform::AddCategory { name, id, category } => {
            for channel in channels.iter_mut() {
                if channel_matches(channel, name, id) {
                    channel.category = Some(category.clone());
                }
            }
        }
        Transform::AddDescription {
            name,
            id,
            description,
        } => {
            for channel in channels.iter_mut() {
                if channel_matches(channel, name, id) {
                    channel.description = Some(description.clone());
                }
            }
        }
        Transform::Rename { name, id, to } => {
            for channel in channels.iter_mut() {
                if channel_matches(channel, name, id) {
                    channel.name = Some(to.clone());
                }
            }
        }
    }
}

fn channel_matches(channel: &Channel, name: &Option<String>, id: &Option<String>) -> bool {
    let name_matches = name
        .as_ref()
        .map(|n| channel.name.as_ref() == Some(n))
        .unwrap_or(true);
    let id_matches = id.as_ref().map(|i| &channel.id == i).unwrap_or(true);

    if name.is_none() && id.is_none() {
        true
    } else {
        (name.is_some() && name_matches) || (id.is_some() && id_matches)
    }
}
