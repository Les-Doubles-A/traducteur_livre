//! Langues proposées dans l'interface : (code ISO, nom anglais pour le prompt, libellé affiché).
//! N'importe quelle autre langue peut être saisie librement ("Autre").

pub const LANGS: &[(&str, &str, &str)] = &[
    ("fr", "French", "Français"),
    ("en", "English", "Anglais"),
    ("es", "Spanish", "Espagnol"),
    ("de", "German", "Allemand"),
    ("it", "Italian", "Italien"),
    ("pt", "Portuguese", "Portugais"),
    ("pt-br", "Brazilian Portuguese", "Portugais (Brésil)"),
    ("nl", "Dutch", "Néerlandais"),
    ("ru", "Russian", "Russe"),
    ("uk", "Ukrainian", "Ukrainien"),
    ("pl", "Polish", "Polonais"),
    ("cs", "Czech", "Tchèque"),
    ("sk", "Slovak", "Slovaque"),
    ("sl", "Slovenian", "Slovène"),
    ("hr", "Croatian", "Croate"),
    ("sr", "Serbian", "Serbe"),
    ("bg", "Bulgarian", "Bulgare"),
    ("ro", "Romanian", "Roumain"),
    ("hu", "Hungarian", "Hongrois"),
    ("el", "Greek", "Grec"),
    ("tr", "Turkish", "Turc"),
    ("sv", "Swedish", "Suédois"),
    ("da", "Danish", "Danois"),
    ("nb", "Norwegian", "Norvégien"),
    ("fi", "Finnish", "Finnois"),
    ("et", "Estonian", "Estonien"),
    ("lv", "Latvian", "Letton"),
    ("lt", "Lithuanian", "Lituanien"),
    ("ar", "Arabic", "Arabe"),
    ("he", "Hebrew", "Hébreu"),
    ("fa", "Persian", "Persan"),
    ("ur", "Urdu", "Ourdou"),
    ("hi", "Hindi", "Hindi"),
    ("bn", "Bengali", "Bengali"),
    ("ta", "Tamil", "Tamoul"),
    ("th", "Thai", "Thaï"),
    ("vi", "Vietnamese", "Vietnamien"),
    ("id", "Indonesian", "Indonésien"),
    ("ms", "Malay", "Malais"),
    ("tl", "Tagalog", "Tagalog"),
    ("zh", "Simplified Chinese", "Chinois (simplifié)"),
    ("zh-tw", "Traditional Chinese", "Chinois (traditionnel)"),
    ("ja", "Japanese", "Japonais"),
    ("ko", "Korean", "Coréen"),
    ("ca", "Catalan", "Catalan"),
    ("eu", "Basque", "Basque"),
    ("ga", "Irish", "Irlandais"),
    ("la", "Latin", "Latin"),
    ("eo", "Esperanto", "Espéranto"),
    ("sw", "Swahili", "Swahili"),
];

pub const AUTO: &str = "auto";

/// Nom anglais utilisé dans le prompt (un code inconnu = texte libre saisi par l'utilisateur).
pub fn prompt_name(code: &str) -> String {
    LANGS.iter().find(|l| l.0 == code).map(|l| l.1.to_string()).unwrap_or_else(|| code.to_string())
}

pub fn label(code: &str) -> String {
    if code == AUTO {
        return "Détection auto".into();
    }
    LANGS.iter().find(|l| l.0 == code).map(|l| l.2.to_string()).unwrap_or_else(|| code.to_string())
}

pub fn is_rtl(code: &str) -> bool {
    matches!(code, "ar" | "he" | "fa" | "ur")
}

/// Code court pour les noms de fichiers et les métadonnées (xml:lang).
pub fn file_code(code: &str) -> String {
    let c: String = code.chars().filter(|c| c.is_alphanumeric() || *c == '-').collect::<String>().to_lowercase();
    if c.is_empty() { "trad".into() } else { c }
}

/// Codes DeepL (cible).
pub fn deepl_target(code: &str) -> String {
    match code {
        "en" => "EN-US".into(),
        "pt" => "PT-PT".into(),
        "pt-br" => "PT-BR".into(),
        "zh" => "ZH-HANS".into(),
        "zh-tw" => "ZH-HANT".into(),
        c => c.to_uppercase(),
    }
}

pub fn deepl_source(code: &str) -> Option<String> {
    if code == AUTO {
        return None;
    }
    Some(code.split('-').next().unwrap_or(code).to_uppercase())
}
