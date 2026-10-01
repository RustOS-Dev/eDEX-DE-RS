//! Keyboard layouts from xkb's evdev.xml (edex-comp applies the chosen one from config.toml).

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayoutInfo {
    pub code: String,
    pub description: String,
}

pub fn layouts(xml_path: &str) -> Vec<LayoutInfo> {
    let Ok(text) = std::fs::read_to_string(xml_path) else {
        return vec![LayoutInfo {
            code: "us".into(),
            description: "English (US)".into(),
        }];
    };
    let Ok(doc) = roxmltree::Document::parse(&text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for layout in doc.descendants().filter(|n| n.has_tag_name("layout")) {
        let Some(cfg) = layout.children().find(|c| c.has_tag_name("configItem")) else {
            continue;
        };
        let name = cfg
            .children()
            .find(|c| c.has_tag_name("name"))
            .and_then(|n| n.text())
            .unwrap_or("")
            .to_string();
        let desc = cfg
            .children()
            .find(|c| c.has_tag_name("description"))
            .and_then(|n| n.text())
            .unwrap_or("")
            .to_string();
        if !name.is_empty() {
            out.push(LayoutInfo {
                code: name,
                description: desc,
            });
        }
    }
    out.sort_by(|a, b| a.description.cmp(&b.description));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_evdev_xml() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("evdev.xml");
        std::fs::write(&p, r#"<?xml version="1.0"?><xkbConfigRegistry><layoutList><layout><configItem><name>us</name><description>English (US)</description></configItem></layout><layout><configItem><name>de</name><description>German</description></configItem></layout></layoutList></xkbConfigRegistry>"#).unwrap();
        let l = layouts(p.to_str().unwrap());
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].code, "us");
        assert_eq!(l[1].description, "German");
    }
}
