/// Prompt templates for each LLM task.
/// `raw_*` functions return the instruction text without model-specific formatting.
/// `gemma_prompt` wraps for local Gemma; OpenRouter uses the raw text directly.

#[cfg(feature = "local-ml")]
fn gemma_prompt(instruction: &str) -> String {
    format!("<start_of_turn>user\n{instruction}<end_of_turn>\n<start_of_turn>model\n")
}

/// Wraps configurable prompts used by existing non-chat local LLM flows.
#[cfg(feature = "local-ml")]
pub fn gemma_wrap(instruction: &str) -> String {
    gemma_prompt(instruction)
}

// ---------------------------------------------------------------------------
// Raw instruction text (model-agnostic)
// ---------------------------------------------------------------------------

pub const OCR_CORRECTION_FORMAT_MARKER: &str = "REGLAS DE FORMATO OCR (OBLIGATORIAS):";

pub const OCR_CORRECTION_FORMAT_CONTRACT: &str = r#"REGLAS DE FORMATO OCR (OBLIGATORIAS):
Estas reglas tienen prioridad sobre cualquier instrucción contradictoria previa del prompt personalizado.
- No elimines, agregues, reordenes ni modifiques etiquetas HTML.
- Conservá todos los atributos HTML, etiquetas de apertura y cierre, y su anidamiento.
- Conservá todo el etiquetado Markdown: encabezados, negritas, cursivas, listas, enlaces, tablas, bloques de código y referencias de imágenes.
- No conviertas HTML a Markdown ni Markdown a HTML.
- No elimines ni modifiques referencias `page/bbox` ni otros destinos de imágenes.
- La única excepción son los saltos de línea claramente producidos por el formato de columnas: uní líneas o bloques que corten una oración según la continuidad gramatical y el contexto, aunque no sean líneas cortas.
- No unas líneas si eso altera una estructura Markdown o HTML, un encabezado, una lista, una tabla, un bloque de código, un enlace, una imagen o un límite real de párrafo.
- No uses la longitud de una línea como único criterio: evaluá también la continuidad gramatical y el contexto.
- No alteres delimitadores o símbolos de HTML o Markdown aunque estén cerca de un error OCR. La única excepción adicional es restituir delimitadores de énfasis Markdown cuando la imagen muestre claramente negrita o cursiva; no modifiques ningún otro delimitador.

La salida debe conservar el HTML, el Markdown, las referencias `page/bbox` y los saltos de línea que no sean artefactos del layout de columnas."#;

pub const OCR_CORRECTION_RECONSTRUCTION_MARKER: &str =
    "REGLAS DE RECONSTRUCCIÓN VISUAL OCRC (OBLIGATORIAS):";

pub const OCR_CORRECTION_RECONSTRUCTION_CONTRACT: &str = r#"REGLAS DE RECONSTRUCCIÓN VISUAL OCRC (OBLIGATORIAS):
También tienen prioridad sobre cualquier instrucción contradictoria previa del prompt personalizado.
- Recorré la página completa en su orden natural de lectura y respetá la secuencia entre columnas, bloques, encabezados y párrafos.
- La separación en párrafos del OCR es provisional: uní bloques separados por líneas en blanco cuando la continuidad gramatical muestre que forman el mismo párrafo o la imagen adjunta lo confirme, aunque no sean líneas cortas. Conservá la separación únicamente cuando el OCR o la imagen muestren un límite real.
- Reconstruí todas las palabras, oraciones y párrafos cortados por columnas usando la imagen adjunta, cuando esté disponible, y la continuidad gramatical; no dejes fragmentos breves aislados si continúan en otro bloque.
- Verificá antes de responder que estén representados el primer y el último texto legible, todos los encabezados y cada sección de la página.
- Restituí negritas o cursivas en Markdown únicamente cuando una imagen adjunta muestre ese énfasis con claridad; sin imagen, conservá intacto todo el etiquetado existente.
- No inventes texto ilegible ni completes contenido que no pueda sostenerse con la imagen disponible o el contexto."#;

pub const DEFAULT_OCR_CORRECTION_PROMPT: &str = r#"Sos un especialista en transcripción de documentos históricos. El siguiente texto fue extraído por OCR de un documento impreso y contiene errores.

Tu tarea:
1. Corregí errores de OCR únicamente dentro del contenido textual visible: sustituciones de caracteres, espacios faltantes, palabras mal leídas y letras incorrectas.
2. Preservá el idioma, estilo y terminología histórica originales. No modernices ni interpretes.
3. Si una palabra o fragmento es dudoso, conservá la versión más probable según el contexto, pero no inventes contenido ausente.
4. No resumas ni reescribas. Mantené el contenido y el nivel de detalle, pero restablecé el orden natural de lectura y unificá los cortes de línea causados por columnas para que no queden oraciones ni párrafos cortados.
5. Si una palabra quedó cortada por un guion de fin de línea, reconstruila únicamente cuando el guion sea un corte de layout y no parte del contenido.

Devolvé únicamente el contenido corregido.
No agregues explicaciones, títulos, comillas, bloques de código ni JSON.
No repitas la consigna.

REGLAS DE FORMATO OCR (OBLIGATORIAS):
Estas reglas tienen prioridad sobre cualquier instrucción contradictoria previa del prompt personalizado.
- No elimines, agregues, reordenes ni modifiques etiquetas HTML.
- Conservá todos los atributos HTML, etiquetas de apertura y cierre, y su anidamiento.
- Conservá todo el etiquetado Markdown: encabezados, negritas, cursivas, listas, enlaces, tablas, bloques de código y referencias de imágenes.
- No conviertas HTML a Markdown ni Markdown a HTML.
- No elimines ni modifiques referencias `page/bbox` ni otros destinos de imágenes.
- La única excepción son los saltos de línea claramente producidos por el formato de columnas: uní líneas o bloques que corten una oración según la continuidad gramatical y el contexto, aunque no sean líneas cortas.
- No unas líneas si eso altera una estructura Markdown o HTML, un encabezado, una lista, una tabla, un bloque de código, un enlace, una imagen o un límite real de párrafo.
- No uses la longitud de una línea como único criterio: evaluá también la continuidad gramatical y el contexto.
- No alteres delimitadores o símbolos de HTML o Markdown aunque estén cerca de un error OCR. La única excepción adicional es restituir delimitadores de énfasis Markdown cuando la imagen muestre claramente negrita o cursiva; no modifiques ningún otro delimitador.

La salida debe conservar el HTML, el Markdown, las referencias `page/bbox` y los saltos de línea que no sean artefactos del layout de columnas.

REGLAS DE RECONSTRUCCIÓN VISUAL OCRC (OBLIGATORIAS):
También tienen prioridad sobre cualquier instrucción contradictoria previa del prompt personalizado.
- Recorré la página completa en su orden natural de lectura y respetá la secuencia entre columnas, bloques, encabezados y párrafos.
- La separación en párrafos del OCR es provisional: uní bloques separados por líneas en blanco cuando la continuidad gramatical muestre que forman el mismo párrafo o la imagen adjunta lo confirme, aunque no sean líneas cortas. Conservá la separación únicamente cuando el OCR o la imagen muestren un límite real.
- Reconstruí todas las palabras, oraciones y párrafos cortados por columnas usando la imagen adjunta, cuando esté disponible, y la continuidad gramatical; no dejes fragmentos breves aislados si continúan en otro bloque.
- Verificá antes de responder que estén representados el primer y el último texto legible, todos los encabezados y cada sección de la página.
- Restituí negritas o cursivas en Markdown únicamente cuando una imagen adjunta muestre ese énfasis con claridad; sin imagen, conservá intacto todo el etiquetado existente.
- No inventes texto ilegible ni completes contenido que no pueda sostenerse con la imagen disponible o el contexto.

Texto OCR:
{text}"#;

pub const DEFAULT_TRIPLETS_PROMPT: &str = r#"Extraé triples semánticos (sujeto-predicado-objeto) de este texto de documento histórico.

Reglas obligatorias:
- Devolvé SOLO un array JSON válido.
- Cada elemento DEBE ser un objeto con EXACTAMENTE estas claves: "subject", "predicate", "object".
- Todos los valores DEBEN ser strings JSON válidos.
- No agregues claves extra.
- No agregues texto antes ni después del array.
- Si no encontrás relaciones confiables, devolvé [].
- Preferí sujetos y objetos completos (sintagmas nominales completos), no fragmentos sueltos, pronombres ni títulos aislados si el referente explícito aparece en el texto.
- Evitá duplicados o variantes mínimas de la misma relación.

Enfocate en relaciones fácticas: quién hizo qué, quién está relacionado con quién, qué pasó dónde y cuándo. Usá los términos exactos del texto. Respondé en el mismo idioma que el texto original (por defecto, español).

Ejemplo válido:
[
  {"subject":"Juan Pérez","predicate":"firmó","object":"el acta"}
]

Texto:
{text}"#;

pub const DEFAULT_SUMMARY_PROMPT: &str = r#"Resumí este texto de documento histórico en un ÚNICO párrafo conciso. El resumen debe:
- Tener entre 10 y 15 líneas
- Preservar nombres propios, fechas, lugares y eventos clave
- Estar escrito en el mismo idioma que el texto original (por defecto, español)
- SIEMPRE terminar con una oración completa que termine en punto

NO superes las 15 líneas. NO cortes a mitad de frase.

Texto:
{text}"#;

pub const DEFAULT_NER_PROMPT: &str = r#"Extraé entidades nombradas del texto histórico. Devolvé SOLO JSON válido, sin markdown. Usá exclusivamente estas categorías: PER, LOC, ORG, DATE, MISC. Formato: [{"value":"...","type":"PER|LOC|ORG|DATE|MISC","start_offset":0,"end_offset":0,"confidence":0.95}]. Si no hay entidades, devolvé []. No inventes entidades ni uses categorías fuera del contrato.

Texto:
{text}"#;

pub fn render_template(template: &str, text: &str) -> String {
    template.replace("{text}", text)
}

pub fn ensure_ocr_correction_contract(prompt: &str) -> String {
    match (
        prompt.contains(OCR_CORRECTION_FORMAT_MARKER),
        prompt.contains(OCR_CORRECTION_RECONSTRUCTION_MARKER),
    ) {
        (true, true) => prompt.to_string(),
        (false, true) => format!("{prompt}\n\n{OCR_CORRECTION_FORMAT_CONTRACT}"),
        (true, false) => format!("{prompt}\n\n{OCR_CORRECTION_RECONSTRUCTION_CONTRACT}"),
        (false, false) => format!(
            "{prompt}\n\n{OCR_CORRECTION_FORMAT_CONTRACT}\n\n{OCR_CORRECTION_RECONSTRUCTION_CONTRACT}"
        ),
    }
}

pub fn with_ocr_image_context(prompt: &str) -> String {
    let prompt = ensure_ocr_correction_contract(prompt);
    format!(
        "{prompt}\n\n\
CONTEXTO VISUAL OCRC (la imagen está adjunta como una entrada separada):\n\
La imagen adjunta corresponde al mismo asset/página que el texto OCR incluido arriba. \
Inspeccioná la imagen de forma independiente y comparala con el texto OCR de arriba. \
La imagen es la fuente de verdad visual para corregir caracteres, palabras y bloques legibles omitidos.\n\
Incluí metadata editorial y texto periférico claramente visible: ciudad, fecha, dateline, encabezados, \
títulos, subtítulos, copetes, pies, firmas, números y rótulos. No inventes contenido ilegible o ausente.\n\
Aplicá íntegramente los contratos obligatorios de formato y reconstrucción incluidos arriba; \
no restrinjas la unión de bloques por la longitud original de sus líneas.\n\n\
Devolvé únicamente el contenido corregido, sin explicar el proceso ni repetir la consigna."
    )
}

#[cfg(feature = "local-ml")]
pub fn raw_ocr_correction(text: &str) -> String {
    render_template(DEFAULT_OCR_CORRECTION_PROMPT, text)
}

#[cfg(feature = "local-ml")]
pub fn raw_extract_entities(text: &str) -> String {
    format!(
        r#"Extraé entidades nombradas de este texto de documento histórico. Devolvé un array JSON donde cada elemento tiene: "value" (el texto de la entidad), "type" (uno de: person, place, date, organization, institution, misc), "confidence" (0.0 a 1.0).

Solo extraé entidades de las que estés seguro. Para fechas, usá el formato original del texto. Respondé en el mismo idioma que el texto original (por defecto, español).

Devolvé SOLO el array JSON, sin explicaciones.

Texto:
{text}"#
    )
}

#[cfg(feature = "local-ml")]
pub fn raw_extract_triples(text: &str) -> String {
    render_template(DEFAULT_TRIPLETS_PROMPT, text)
}

pub fn raw_consolidate_entities(text: &str, candidate_entities_json: &str) -> String {
    format!(
        r#"Sos una capa de validación y mejora para un pipeline NER histórico.

Recibís:
1. El texto original.
2. Una lista preliminar de entidades detectadas por el pipeline NER actual.

Tu tarea:
- Revisá las entidades preliminares.
- Corregí OCR evidente dentro del valor de la entidad cuando el contexto lo haga claro.
- Normalizá variantes obvias del mismo nombre si corresponden, pero sin modernizar el texto.
- Eliminá falsos positivos.
- Agregá entidades relevantes que el NER no haya detectado.
- Mantené un tipado consistente usando SOLO: person, place, date, organization, institution, misc.
- No incluyas duplicados ni variantes mínimas de la misma entidad.
- Priorizá entidades concretas y útiles para búsqueda/exploración.

Reglas de salida:
- Devolvé SOLO un array JSON válido.
- Cada elemento debe tener EXACTAMENTE estas claves: "value", "type", "confidence".
- "value" debe ser un string.
- "type" debe ser uno de: person, place, date, organization, institution, misc.
- "confidence" debe ser un número entre 0.0 y 1.0.
- No agregues texto fuera del JSON.
- Si no hay entidades válidas, devolvé [].

Entidades preliminares:
{candidate_entities_json}

Texto:
{text}"#
    )
}

#[cfg(feature = "local-ml")]
pub fn consolidate_entities(text: &str, candidate_entities_json: &str) -> String {
    gemma_prompt(&raw_consolidate_entities(text, candidate_entities_json))
}

#[cfg(feature = "local-ml")]
pub fn raw_summarize(text: &str) -> String {
    render_template(DEFAULT_SUMMARY_PROMPT, text)
}

pub fn raw_classify(text: &str, categories: &[String]) -> String {
    let cats = categories.join(", ");
    format!(
        r#"Clasificá este documento histórico en una o más de estas categorías: {cats}

Devolvé un array JSON de objetos con: "category" (de la lista arriba), "confidence" (0.0 a 1.0). Respondé en el mismo idioma que el texto original (por defecto, español).

Devolvé SOLO el array JSON, sin explicaciones.

Texto:
{text}"#
    )
}

pub fn raw_question_answer(question: &str, context: &str) -> String {
    format!(
        r#"Respondé la siguiente pregunta basándote SOLO en los fragmentos de documento provistos. Si la respuesta no se puede determinar del contexto, decilo explícitamente. Respondé en el mismo idioma que la pregunta (por defecto, español).

Contexto:
{context}

Pregunta: {question}"#
    )
}

/// Prompt del chat RAG: instrucciones de citación `[n]`, fragmentos numerados,
/// historial (opcional) y la pregunta. Devuelve texto crudo (sin wrapping de
/// modelo); el motor local aplica el template del GGUF y OpenRouter lo usa tal
/// cual.
pub fn raw_rag_answer(question: &str, context: &str, history: &str) -> String {
    let history_block = if history.trim().is_empty() {
        String::new()
    } else {
        format!("Conversación previa (solo para interpretar la pregunta, NO es una fuente):\n{history}\n\n")
    };

    format!(
        r#"Sos un asistente de investigación académica especializado en fuentes históricas y de archivo.

Reglas obligatorias:
1. Respondé EXCLUSIVAMENTE con información presente en los fragmentos numerados provistos.
2. Citá cada afirmación con el número del fragmento que la respalda usando el formato [n]. Toda afirmación debe llevar al menos una cita.
3. Si la respuesta no se puede determinar a partir de los fragmentos, decilo explícitamente y NO inventes contenido.
4. Distinguí con claridad lo que dicen las fuentes de lo que es inferencia tuya; si inferís algo, indicalo.
5. Sé preciso con nombres, fechas, lugares y cifras: usá los términos exactos de los fragmentos.
6. Respondé en el mismo idioma de la pregunta (por defecto, español).

Fragmentos:
{context}

{history_block}Pregunta: {question}"#
    )
}

/// Prompt conversacional del chat de investigación. No recibe fragmentos ni
/// produce citas: el historial solo aporta continuidad al diálogo.
pub fn raw_direct_chat_answer(question: &str, history: &str) -> String {
    let history_block = if history.trim().is_empty() {
        String::new()
    } else {
        format!(
            "Conversación previa (solo como contexto conversacional, no como evidencia documental):\n{history}\n\n"
        )
    };

    format!(
        r#"Sos el asistente del chat de investigación.

Reglas obligatorias:
1. Respondé en el mismo idioma del mensaje actual (por defecto, español).
2. Usá la conversación previa únicamente para mantener el contexto del diálogo.
3. No incluyas citas con formato [n] ni una sección llamada "Fuentes".
4. No afirmes que consultaste, encontraste o verificaste documentos para esta respuesta.
5. Si preguntan por tus capacidades o por el funcionamiento del chat, explicá con precisión que podés conversar y que las preguntas que requieren evidencia se buscan en la base de transcripciones y documentos OCR, mostrando las fuentes citadas cuando corresponde.
6. Sé directo, cordial y veraz; no inventes capacidades ni resultados documentales.

{history_block}Mensaje actual:
{question}"#
    )
}

// ---------------------------------------------------------------------------
// Gemma-wrapped prompts (used by local LlmEngine)
// ---------------------------------------------------------------------------

#[cfg(feature = "local-ml")]
pub fn ocr_correction(text: &str) -> String {
    gemma_prompt(&raw_ocr_correction(text))
}

#[cfg(feature = "local-ml")]
pub fn extract_entities(text: &str) -> String {
    gemma_prompt(&raw_extract_entities(text))
}

#[cfg(feature = "local-ml")]
pub fn extract_triples(text: &str) -> String {
    gemma_prompt(&raw_extract_triples(text))
}

#[cfg(feature = "local-ml")]
pub fn summarize(text: &str) -> String {
    gemma_prompt(&raw_summarize(text))
}

#[cfg(feature = "local-ml")]
pub fn classify(text: &str, categories: &[String]) -> String {
    gemma_prompt(&raw_classify(text, categories))
}

#[cfg(feature = "local-ml")]
pub fn question_answer(question: &str, context: &str) -> String {
    gemma_prompt(&raw_question_answer(question, context))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multimodal_ocr_context_preserves_markup_rules() {
        let prompt = with_ocr_image_context("OCR borrador:\ntexto");

        assert!(prompt.contains("imagen adjunta"));
        assert!(prompt.contains("fuente de verdad visual"));
        assert!(prompt.contains("ciudad, fecha, dateline"));
        assert!(prompt.contains("bloques legibles omitidos"));
        assert!(prompt.contains("OCR borrador:\ntexto"));
        assert!(prompt.contains("prioridad sobre cualquier instrucción contradictoria"));
        assert!(prompt.contains("etiquetas HTML"));
        assert!(prompt.contains("Markdown"));
        assert!(!prompt.contains("texto plano sin HTML ni Markdown"));
        assert!(prompt.contains("aunque no sean líneas cortas"));
        assert!(prompt.contains(
            "La única excepción adicional es restituir delimitadores de énfasis Markdown"
        ));
        assert!(!prompt.contains("pueden unirse cuando sean líneas muy cortas"));
        assert!(!prompt.contains("uní líneas muy cortas"));
    }

    #[test]
    fn custom_ocr_prompt_gets_the_canonical_format_contract_once() {
        let custom = "Custom OCR: {text}\nNO agregues HTML ni Markdown.";
        let with_contract = ensure_ocr_correction_contract(custom);

        assert!(with_contract.starts_with(custom));
        assert_eq!(
            with_contract.matches(OCR_CORRECTION_FORMAT_MARKER).count(),
            1
        );
        assert!(with_contract.contains("etiquetas HTML"));
        assert!(with_contract.contains("NO agregues HTML ni Markdown."));
        assert!(with_contract.contains("Markdown"));
        assert_eq!(
            with_contract
                .matches("REGLAS DE RECONSTRUCCIÓN VISUAL OCRC (OBLIGATORIAS):")
                .count(),
            1
        );
        assert!(with_contract.contains("orden natural de lectura"));
        assert!(with_contract.contains(
            "También tienen prioridad sobre cualquier instrucción contradictoria previa"
        ));
        assert!(with_contract.contains("La separación en párrafos del OCR es provisional"));
        assert!(with_contract.contains("aunque no sean líneas cortas"));
        assert!(with_contract.contains(
            "La única excepción adicional es restituir delimitadores de énfasis Markdown"
        ));
        assert!(with_contract
            .contains("la imagen adjunta, cuando esté disponible, y la continuidad gramatical"));
        assert!(!with_contract.contains("usando la imagen adjunta y la continuidad gramatical"));
    }

    #[test]
    fn persisted_ocr_prompt_with_format_contract_gets_the_reconstruction_contract() {
        let persisted = format!("Prompt guardado: {{text}}\n\n{OCR_CORRECTION_FORMAT_CONTRACT}");
        let with_contract = ensure_ocr_correction_contract(&persisted);

        assert_eq!(
            with_contract.matches(OCR_CORRECTION_FORMAT_MARKER).count(),
            1
        );
        assert_eq!(
            with_contract
                .matches("REGLAS DE RECONSTRUCCIÓN VISUAL OCRC (OBLIGATORIAS):")
                .count(),
            1
        );
    }

    #[test]
    fn default_ocr_prompt_contains_the_text_placeholder_and_format_contract() {
        assert!(DEFAULT_OCR_CORRECTION_PROMPT.contains("{text}"));
        assert!(DEFAULT_OCR_CORRECTION_PROMPT.contains(OCR_CORRECTION_FORMAT_MARKER));
        assert!(DEFAULT_OCR_CORRECTION_PROMPT.contains(
            "Mantené el contenido y el nivel de detalle, pero restablecé el orden natural de lectura"
        ));
        assert!(!DEFAULT_OCR_CORRECTION_PROMPT
            .contains("Mantené el contenido, el orden y el nivel de detalle"));
        assert!(DEFAULT_OCR_CORRECTION_PROMPT
            .contains("Recorré la página completa en su orden natural de lectura"));
        assert!(DEFAULT_OCR_CORRECTION_PROMPT
            .contains("Reconstruí todas las palabras, oraciones y párrafos cortados por columnas"));
        assert!(DEFAULT_OCR_CORRECTION_PROMPT
            .contains("la imagen adjunta, cuando esté disponible, y la continuidad gramatical"));
        assert!(DEFAULT_OCR_CORRECTION_PROMPT
            .contains("el primer y el último texto legible, todos los encabezados y cada sección"));
        assert!(DEFAULT_OCR_CORRECTION_PROMPT.contains(
            "Restituí negritas o cursivas en Markdown únicamente cuando una imagen adjunta muestre ese énfasis con claridad"
        ));
        assert!(DEFAULT_OCR_CORRECTION_PROMPT.contains(
            "La única excepción adicional es restituir delimitadores de énfasis Markdown"
        ));
        assert!(DEFAULT_OCR_CORRECTION_PROMPT.contains(
            "También tienen prioridad sobre cualquier instrucción contradictoria previa"
        ));
        assert!(DEFAULT_OCR_CORRECTION_PROMPT
            .contains("La separación en párrafos del OCR es provisional"));
        assert!(DEFAULT_OCR_CORRECTION_PROMPT.contains("aunque no sean líneas cortas"));
    }
}
