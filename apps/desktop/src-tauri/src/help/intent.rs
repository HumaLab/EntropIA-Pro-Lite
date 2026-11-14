//! Action-request detection for the in-app help assistant (S17, D15).
//!
//! A question that asks the assistant to DO something — an imperative, an
//! infinitive request, or a «podés / puedes» / «can you» ask — never gets a
//! local FAQ answer: the request goes to the model, which T20–T22 will give
//! tools. A question that asks HOW («¿Cómo…?», «¿Qué es…?», «¿Dónde…?»)
//! still answers locally. Pure and deterministic: accent-folded,
//! case-insensitive, no I/O, so every rule is a unit test.

use super::text;

/// Spanish verb forms that start an action request, folded (so voseo
/// «creá» and tuteo «crea» meet here as one «crea»). Imperatives and
/// bare infinitive requests both count: «Creá…», «Crea…» and «crear…»
/// all act. Verbs of telling (`decir`, `explicar`, `contar`,
/// `responder`) are deliberately absent: «Decime cómo…» asks for an
/// explanation, and the FAQ/manual answers ARE explanations. «Parar» is
/// handled in [`is_action_request`]: at the start, «para» + infinitive
/// (or «para qué») is a purpose clause, not a stop command.
const ES_IMPERATIVES: &[&str] = &[
    "abre",
    "abri",
    "agrega",
    "anade",
    "analiza",
    "archiva",
    "borra",
    "busca",
    "cambia",
    "cancela",
    "carga",
    "cierra",
    "combina",
    "comprueba",
    "conecta",
    "configura",
    "continua",
    "convierte",
    "converti",
    "copia",
    "corrige",
    "corregi",
    "crea",
    "da",
    "descarga",
    "deten",
    "detene",
    "digitaliza",
    "divide",
    "dividi",
    "duplica",
    "elimina",
    "empieza",
    "envia",
    "escanea",
    "escribe",
    "escribi",
    "exporta",
    "extrae",
    "filtra",
    "finaliza",
    "fusiona",
    "genera",
    "graba",
    "guarda",
    "hace",
    "haz",
    "importa",
    "indexa",
    "inicia",
    "investiga",
    "lee",
    "manda",
    "marca",
    "mostra",
    "mueve",
    "move",
    "muestra",
    "ordena",
    "organiza",
    "pasa",
    "pausa",
    "pega",
    "pon",
    "pone",
    "procesa",
    "prueba",
    "quita",
    "reanuda",
    "recorta",
    "renombra",
    "repara",
    "repite",
    "repeti",
    "reproduce",
    "reproduci",
    "resume",
    "resumi",
    "rota",
    "saca",
    "selecciona",
    "separa",
    "sincroniza",
    "sube",
    "subi",
    "termina",
    "traduce",
    "traduci",
    "transcribe",
    "transcribi",
    "une",
    "uni",
    "verifica",
];

/// The infinitives behind [`ES_IMPERATIVES`]: «crear una colección…»
/// acts, and so does «podés crear…» / «puedes crear…».
const ES_INFINITIVES: &[&str] = &[
    "abrir",
    "agregar",
    "anadir",
    "analizar",
    "archivar",
    "borrar",
    "buscar",
    "cambiar",
    "cancelar",
    "cargar",
    "cerrar",
    "combinar",
    "comprobar",
    "conectar",
    "configurar",
    "continuar",
    "convertir",
    "copiar",
    "corregir",
    "crear",
    "dar",
    "descargar",
    "detener",
    "digitalizar",
    "dividir",
    "duplicar",
    "eliminar",
    "empezar",
    "enviar",
    "escanear",
    "escribir",
    "exportar",
    "extraer",
    "filtrar",
    "finalizar",
    "fusionar",
    "generar",
    "grabar",
    "guardar",
    "hacer",
    "importar",
    "indexar",
    "iniciar",
    "investigar",
    "leer",
    "mandar",
    "marcar",
    "mostrar",
    "mover",
    "ordenar",
    "organizar",
    "parar",
    "pasar",
    "pausar",
    "pegar",
    "poner",
    "procesar",
    "probar",
    "quitar",
    "reanudar",
    "recortar",
    "renombrar",
    "reparar",
    "repetir",
    "reproducir",
    "resumir",
    "rotar",
    "sacar",
    "seleccionar",
    "separar",
    "sincronizar",
    "subir",
    "terminar",
    "traducir",
    "transcribir",
    "unir",
    "verificar",
];

/// English verbs that start an action request: «Create…», «Delete…».
/// «Do» is deliberately absent: «Do I need…?» asks, it does not order.
const EN_ACTION_VERBS: &[&str] = &[
    "add",
    "analyze",
    "archive",
    "cancel",
    "check",
    "close",
    "combine",
    "configure",
    "connect",
    "convert",
    "copy",
    "create",
    "delete",
    "download",
    "duplicate",
    "execute",
    "export",
    "extract",
    "fix",
    "generate",
    "import",
    "index",
    "load",
    "make",
    "merge",
    "move",
    "open",
    "organize",
    "paste",
    "pause",
    "process",
    "read",
    "remove",
    "rename",
    "reproduce",
    "run",
    "save",
    "scan",
    "search",
    "select",
    "send",
    "split",
    "start",
    "stop",
    "summarize",
    "sync",
    "transcribe",
    "translate",
    "update",
    "upload",
    "write",
];

/// Second-person asks in Spanish: «podés», «podrías», «podría», «puedes».
const ES_MODALS: &[&str] = &["podes", "podrias", "podria", "puedes"];

/// Second-person asks in English: «can you», «could you», «would you».
const EN_MODALS: &[&str] = &["can", "could", "would"];

/// Enclitic pronouns stuck to an imperative or infinitive («creame»,
/// «borrarlo», «decime»): stripped before the verb check so «Creame
/// una colección» still acts.
const ENCLITICS: &[&str] = &[
    "me", "te", "se", "nos", "lo", "la", "los", "las", "les", "le",
];

/// True when the question asks the assistant to DO something rather than
/// asking how: Spanish imperatives (voseo and tuteo) and infinitive
/// requests at the start or after «por favor» / «please», «podés /
/// podrías / puedes» + infinitive, plus the English counterparts.
/// Accent-folded and case-insensitive. Everything else — «¿Cómo…?»,
/// «¿Qué es…?», «¿Dónde…?», verbs of telling — is not an action.
pub(crate) fn is_action_request(question: &str) -> bool {
    let folded = text::fold(question);
    let words: Vec<&str> = folded
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    let words = skip_politeness(&words);
    if words.is_empty() {
        return false;
    }
    // «¿Podés crear…?», «¿podrías importar…?», «¿puedes renombrar…?»:
    // the modal plus an action infinitive acts. «Podés decirme cómo…»
    // asks for an explanation instead, so it stays local.
    if ES_MODALS.contains(&words[0]) {
        let rest = skip_politeness(words.get(1..).unwrap_or_default());
        return rest
            .first()
            .is_some_and(|word| is_action_infinitive(strip_enclitic(word)));
    }
    // «Can you create…?», «Could you rename…?»: the same rule in
    // English. «Can I…?» asks permission, so it stays local.
    if EN_MODALS.contains(&words[0]) {
        let mut rest = words.get(1..).unwrap_or_default();
        if rest.first() == Some(&"you") {
            rest = &rest[1..];
        }
        let rest = skip_politeness(rest);
        return rest
            .first()
            .is_some_and(|word| EN_ACTION_VERBS.contains(word));
    }
    let first = strip_enclitic(words[0]);
    // «Pará el lote» acts, but «para crear…» / «¿para qué…?» is a purpose
    // clause asking how or why, not a stop command.
    if words[0] == "para" {
        return match words.get(1) {
            None => true,
            Some(next) => *next != "que" && !is_action_infinitive(next),
        };
    }
    // The raw word first: «delete» is English for erase, not Spanish
    // «dele» + enclitic, so it must match before any stripping.
    is_action_form(words[0]) || is_action_form(first)
}

/// The verb check behind [`is_action_request`]: a Spanish infinitive or
/// imperative form, or an English action verb («Create…», «Delete…»).
fn is_action_form(word: &str) -> bool {
    is_action_infinitive(word) || ES_IMPERATIVES.contains(&word) || EN_ACTION_VERBS.contains(&word)
}

/// The infinitive check behind the modal rule («podés crear…»).
fn is_action_infinitive(word: &str) -> bool {
    ES_INFINITIVES.contains(&word)
}

/// Strips one enclitic pronoun («creame» → «crea»); shorter stems stay
/// whole so «vale» and «dale» never become «va» and «da».
fn strip_enclitic(word: &str) -> &str {
    for suffix in ENCLITICS {
        if let Some(stem) = word.strip_suffix(suffix) {
            if stem.chars().count() >= 3 {
                return stem;
            }
        }
    }
    word
}

/// Skips a leading politeness wrapper («por favor, …», «please …»).
fn skip_politeness<'a>(words: &'a [&'a str]) -> &'a [&'a str] {
    if words.first() == Some(&"please") {
        return &words[1..];
    }
    if words.first() == Some(&"por") && words.get(1) == Some(&"favor") {
        return &words[2..];
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_sentence_is_an_action_not_a_question() {
        // L33: «Creá una nueva colección con el nombre 'SIMAPE'» got the
        // local FAQ steps instead of creating it.
        assert!(is_action_request(
            "Creá una nueva colección con el nombre 'SIMAPE'"
        ));
    }

    #[test]
    fn spanish_imperatives_voseo_and_tuteo_act() {
        for question in [
            "Creá una colección",
            "Crea una colección",
            "Borrá esta página",
            "Borra la imagen",
            "Eliminá el documento",
            "Elimina el archivo",
            "Importá este PDF",
            "Importa el archivo",
            "Renombrá la colección",
            "Renombra el documento",
            "Cambiá el modelo",
            "Agregá un documento",
            "Mové el archivo a otra colección",
            "Procesá este lote",
            "Hacé OCR de esta página",
            "Haz OCR de la imagen",
            "Hace OCR de la imagen",
            "Transcribí este audio",
            "Transcribe el audio",
            "Buscá en mis documentos la palabra puerto",
            "Busca la palabra puerto",
            "Abrí el documento",
            "Guardá los cambios",
            "Pará el lote",
        ] {
            assert!(is_action_request(question), "{question:?} must act");
        }
    }

    #[test]
    fn spanish_infinitive_requests_act() {
        for question in [
            "crear una colección llamada Prensa",
            "borrar esta página",
            "importar este pdf",
            "transcribir el audio",
        ] {
            assert!(is_action_request(question), "{question:?} must act");
        }
    }

    #[test]
    fn por_favor_requests_act() {
        for question in [
            "Por favor creá una colección",
            "por favor, importá este pdf",
            "Please create a collection",
        ] {
            assert!(is_action_request(question), "{question:?} must act");
        }
    }

    #[test]
    fn poderes_requests_with_an_action_verb_act() {
        for question in [
            "Podés crear una colección?",
            "¿Podés crear una colección?",
            "Podes crear una coleccion",
            "¿Podrías importar este pdf?",
            "¿Puedes renombrar la colección?",
            "Podés por favor crear una colección",
            "¿Podría crear una colección?",
        ] {
            assert!(is_action_request(question), "{question:?} must act");
        }
    }

    #[test]
    fn english_imperatives_and_can_you_act() {
        for question in [
            "Create a new collection",
            "Delete this document",
            "Rename the collection",
            "Import this PDF",
            "Can you create a collection?",
            "Could you rename it?",
            "Would you summarize this document?",
            "Can you please create a collection?",
        ] {
            assert!(is_action_request(question), "{question:?} must act");
        }
    }

    #[test]
    fn enclitic_pronouns_still_act() {
        for question in [
            "Creame una colección",
            "Borralo",
            "Abrilo",
            "Pasame el audio a texto",
            "¿Podés crearme una colección?",
        ] {
            assert!(is_action_request(question), "{question:?} must act");
        }
    }

    #[test]
    fn how_to_questions_stay_local() {
        for question in [
            "¿Cómo creo una colección?",
            "¿Cómo hago OCR?",
            "¿Qué es una colección?",
            "¿Dónde está el botón Guardar cambios?",
            "¿Donde esta el boton Guardar cambios?",
            "How do I create a collection?",
            "What is a collection?",
            "Where is the save button?",
            "Do I need an OpenRouter key?",
            "¿Para qué sirve una colección?",
            "Para crear una colección, ¿qué hago?",
            "Decime cómo creo una colección",
            "¿Podés decirme cómo importar un PDF?",
            "Can I create a collection?",
        ] {
            assert!(!is_action_request(question), "{question:?} must stay local");
        }
    }

    #[test]
    fn empty_questions_do_not_act() {
        for question in ["", "   ", "¿...?", "¿?", "123", "por favor"] {
            assert!(!is_action_request(question), "{question:?} must stay local");
        }
    }
}
