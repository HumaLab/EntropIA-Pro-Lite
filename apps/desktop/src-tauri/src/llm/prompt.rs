/// Prompt templates for each LLM task.
/// `raw_*` functions return the instruction text without model-specific formatting.
/// `gemma_prompt` wraps for local Gemma; OpenRouter uses the raw text directly.

#[cfg(feature = "local-ml")]
fn gemma_prompt(instruction: &str) -> String {
    format!("<start_of_turn>user\n{instruction}<end_of_turn>\n<start_of_turn>model\n")
}

/// Public Gemma wrapper for callers (e.g. the RAG pipeline) that build a raw
/// instruction string themselves and need to wrap it for the local Gemma
/// engine. Identical formatting to the task-specific `*_prompt` helpers.
#[cfg(feature = "local-ml")]
pub fn gemma_wrap(instruction: &str) -> String {
    gemma_prompt(instruction)
}

// ---------------------------------------------------------------------------
// Raw instruction text (model-agnostic)
// ---------------------------------------------------------------------------

pub fn raw_ocr_correction(text: &str) -> String {
    format!(
        r#"Sos un especialista en transcripción de documentos históricos. El siguiente texto fue extraído por OCR de un documento impreso y contiene errores.

Tu tarea:
1. Corregí errores de OCR: sustituciones de caracteres, espacios faltantes, palabras garabateadas, letras mal leídas.
2. Unificá líneas rotas: mergeá líneas que fueron divididas por el layout en columnas o guiones en oraciones y párrafos completos. NO conserves saltos de línea que provienen del layout en columnas — reconstruí el flujo de lectura natural.
3. Ignorá los cortes de columnas de impresión: el texto viene de layouts multi-columna. Mergeá el texto de diferentes columnas en un orden de lectura coherente.
4. Preservá el idioma, estilo y terminología histórica originales. No modernices ni interpretes.
5. Si una palabra o fragmento es dudoso, conservá la versión más probable según el contexto, pero NO inventes contenido ausente.
6. No resumas ni reescribas: corregí el OCR, pero mantené el contenido, el orden de lectura y el nivel de detalle del original.
7. Si una palabra quedó cortada por guion de fin de línea, reconstruila; si el guion pertenece realmente al contenido, conserválo.

Devolvé SOLO el texto corregido y unificado con saltos de párrafo apropiados.
NO agregues explicaciones, títulos, comillas, markdown, bloques de código ni JSON.
NO repitas la consigna.

Texto OCR:
{text}"#
    )
}

pub fn raw_extract_entities(text: &str) -> String {
    format!(
        r#"Extraé entidades nombradas de este texto de documento histórico. Devolvé un array JSON donde cada elemento tiene: "value" (el texto de la entidad), "type" (uno de: person, place, date, organization, institution, misc), "confidence" (0.0 a 1.0).

Solo extraé entidades de las que estés seguro. Para fechas, usá el formato original del texto. Respondé en el mismo idioma que el texto original (por defecto, español).

Devolvé SOLO el array JSON, sin explicaciones.

Texto:
{text}"#
    )
}

pub fn raw_extract_triples(text: &str) -> String {
    format!(
        r#"Extraé triples semánticos (sujeto-predicado-objeto) de este texto de documento histórico.

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
  {{"subject":"Juan Pérez","predicate":"firmó","object":"el acta"}}
]

Texto:
{text}"#
    )
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

pub fn raw_summarize(text: &str) -> String {
    format!(
        r#"Resumí este texto de documento histórico en un ÚNICO párrafo conciso. El resumen debe:
- Tener entre 10 y 15 líneas
- Preservar nombres propios, fechas, lugares y eventos clave
- Estar escrito en el mismo idioma que el texto original (por defecto, español)
- SIEMPRE terminar con una oración completa que termine en punto

NO superes las 15 líneas. NO cortes a mitad de frase.

Texto:
{text}"#
    )
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
/// modelo); el motor local lo envuelve con `gemma_wrap` y OpenRouter lo usa tal
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
