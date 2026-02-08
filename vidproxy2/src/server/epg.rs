use chrono::{DateTime, Duration, Utc};

use crate::channel::ChannelEntry;
use crate::engine::manifest::Source;

use super::images::ImageCache;

/// Format a `DateTime<Utc>` as an XMLTV timestamp (`YYYYMMDDHHmmSS +0000`).
fn format_xmltv_time(dt: &DateTime<Utc>) -> String {
    dt.format("%Y%m%d%H%M%S %z").to_string()
}

/// Generate an XMLTV EPG document from a list of channel entries.
///
/// Pure function apart from image cache registration for programme icons.
pub async fn generate_epg(
    channels: &[ChannelEntry],
    source: &Source,
    base_url: &str,
    image_cache: &ImageCache,
) -> String {
    let source_id = &source.id;

    let lang_attr = source
        .language
        .as_ref()
        .map(|l| format!(" lang=\"{}\"", escape_xml(l)))
        .unwrap_or_default();

    let now = Utc::now();
    let start_of_day = now.date_naive().and_hms_opt(0, 0, 0).unwrap();
    let start = start_of_day.and_utc();

    let mut channel_elements = String::new();
    let mut programmes = String::new();

    for entry in channels {
        let channel_name = entry.channel.name.as_deref().unwrap_or(&entry.channel.id);
        let channel_id = format!("{}:{}", source_id, entry.channel.id);

        let icon_element = if entry.channel.image.is_some() {
            format!(
                "    <icon src=\"{}/{}/{}/image\"/>\n",
                base_url, source_id, entry.channel.id
            )
        } else {
            String::new()
        };

        channel_elements.push_str(&format!(
            "  <channel id=\"{id}\">\n\
             \x20   <display-name{lang}>{name}</display-name>\n\
             {icon}\
             \x20 </channel>\n",
            id = escape_xml(&channel_id),
            name = escape_xml(channel_name),
            lang = lang_attr,
            icon = icon_element,
        ));

        let category_element = entry
            .channel
            .category
            .as_ref()
            .map(|c| format!("    <category{}>{}</category>\n", lang_attr, escape_xml(c)))
            .unwrap_or_default();

        if entry.programmes.is_empty() {
            let desc = entry
                .channel
                .description
                .as_deref()
                .unwrap_or("Live broadcast");

            for day in 0..7 {
                let day_start = start + Duration::days(day);
                let day_end = day_start + Duration::days(1);

                programmes.push_str(&format!(
                    "  <programme start=\"{start}\" stop=\"{stop}\" channel=\"{id}\">\n\
                     \x20   <title{lang}>{name}</title>\n\
                     \x20   <desc{lang}>{desc}</desc>\n\
                     {category}\
                     \x20 </programme>\n",
                    start = format_xmltv_time(&day_start),
                    stop = format_xmltv_time(&day_end),
                    id = escape_xml(&channel_id),
                    category = category_element,
                    name = escape_xml(channel_name),
                    desc = escape_xml(desc),
                    lang = lang_attr,
                ));
            }
        } else {
            for programme in &entry.programmes {
                let desc_element = programme
                    .description
                    .as_ref()
                    .map(|d| format!("    <desc{}>{}</desc>\n", lang_attr, escape_xml(d)))
                    .unwrap_or_default();

                let category_elements: String = if programme.genres.is_empty() {
                    category_element.clone()
                } else {
                    programme
                        .genres
                        .iter()
                        .map(|g| {
                            format!("    <category{}>{}</category>\n", lang_attr, escape_xml(g))
                        })
                        .collect()
                };

                let episode_element = match (&programme.season, &programme.episode) {
                    (Some(s), Some(e)) => {
                        format!(
                            "    <episode-num system=\"onscreen\">S{}E{}</episode-num>\n",
                            s, e
                        )
                    }
                    (None, Some(e)) => {
                        format!(
                            "    <episode-num system=\"onscreen\">E{}</episode-num>\n",
                            e
                        )
                    }
                    _ => String::new(),
                };

                let prog_icon = if let Some(url) = &programme.image {
                    let image_id = image_cache.register_proxy_url(url).await;
                    format!("    <icon src=\"{}/i/{}\"/>\n", base_url, image_id)
                } else {
                    String::new()
                };

                programmes.push_str(&format!(
                    "  <programme start=\"{start}\" stop=\"{stop}\" channel=\"{id}\">\n\
                     \x20   <title{lang}>{title}</title>\n\
                     {desc}\
                     {categories}\
                     {episode}\
                     {icon}\
                     \x20 </programme>\n",
                    start = format_xmltv_time(&programme.start_time),
                    stop = format_xmltv_time(&programme.end_time),
                    id = escape_xml(&channel_id),
                    title = escape_xml(&programme.title),
                    lang = lang_attr,
                    desc = desc_element,
                    categories = category_elements,
                    episode = episode_element,
                    icon = prog_icon,
                ));
            }
        }
    }

    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE tv SYSTEM \"xmltv.dtd\">\n\
         <tv generator-info-name=\"vidproxy\">\n\
         {channels}\
         {programmes}\
         </tv>\n",
        channels = channel_elements,
        programmes = programmes,
    )
}

pub fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
