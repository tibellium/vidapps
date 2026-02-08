use crate::channel::ChannelEntry;
use crate::engine::manifest::Source;

use super::epg::escape_xml;

/// Generate an M3U playlist from a list of channel entries.
///
/// Pure function — no server state needed.
pub fn generate_m3u(channels: &[ChannelEntry], source: &Source, base_url: &str) -> String {
    let source_id = &source.id;

    let mut playlist = format!("#EXTM3U url-tvg=\"{}/{}/epg.xml\"\n", base_url, source_id);

    for entry in channels {
        let channel_name = entry.channel.name.as_deref().unwrap_or(&entry.channel.id);

        let logo_attr = if entry.channel.image.is_some() {
            format!(
                " tvg-logo=\"{}/{}/{}/image\"",
                base_url, source_id, entry.channel.id
            )
        } else {
            String::new()
        };

        let country_attr = source
            .country
            .as_ref()
            .map(|c| format!(" tvg-country=\"{}\"", escape_xml(c)))
            .unwrap_or_default();

        let language_attr = source
            .language
            .as_ref()
            .map(|l| format!(" tvg-language=\"{}\"", escape_xml(l)))
            .unwrap_or_default();

        let channel_id = format!("{}:{}", source_id, entry.channel.id);

        let group = entry.channel.category.as_ref().unwrap_or(&source.name);

        playlist.push_str(&format!(
            "#EXTINF:-1 tvg-id=\"{id}\" tvg-name=\"{name}\" tvg-type=\"live\" group-title=\"{group}\"{logo}{country}{language},{name}\n\
             {base_url}/{source}/{channel}/playlist.m3u8\n",
            id = escape_xml(&channel_id),
            name = escape_xml(channel_name),
            group = escape_xml(group),
            logo = logo_attr,
            country = country_attr,
            language = language_attr,
            base_url = base_url,
            source = source_id,
            channel = entry.channel.id,
        ));
    }

    playlist
}
