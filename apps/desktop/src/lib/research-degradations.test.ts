import { afterEach, describe, expect, it } from 'vitest'

import { locale } from './i18n'
import { describeRoleWarnings } from './research-degradations'

afterEach(() => {
  locale.set('es')
})

describe('describeRoleWarnings', () => {
  it('explains an unreadable answer in plain words and keeps the raw error as detail', () => {
    const error = 'Salida inválida de asistente_redaccion: la salida no contiene JSON recuperable'

    expect(describeRoleWarnings([{ role: 'asistente_redaccion', error, times: 1 }])).toEqual([
      {
        message:
          'Redacción de secciones: el modelo devolvió una respuesta con un formato que el programa no entiende, así que ese paso siguió sin su resultado. Esa parte del informe puede estar incompleta; probá con un modelo más capaz.',
        detail: error,
      },
    ])
  })

  it('tells a cut-off answer apart and says how many times it happened', () => {
    const error =
      'Salida inválida de asistente_validador: la respuesta del modelo se cortó al llegar al límite de salida (16384 tokens)'

    expect(describeRoleWarnings([{ role: 'asistente_validador', error, times: 3 }])).toEqual([
      {
        message:
          'Verificación de afirmaciones: la respuesta del modelo se cortó por largo (3 veces), así que ese paso siguió sin su resultado. Esa parte del informe puede estar incompleta: subí el «Tope de salida» en Configuración › Agente (era de 16384 tokens) o usá un modelo que admita respuestas más largas.',
        detail: error,
      },
    ])
  })

  it('suggests raising the output cap without a number when the engine did not say it', () => {
    const error =
      'Salida inválida de redactor: la respuesta del modelo se cortó al llegar al límite de salida'

    expect(describeRoleWarnings([{ role: 'redactor', error }])[0]!.message).toBe(
      'Redacción del informe: la respuesta del modelo se cortó por largo, así que ese paso siguió sin su resultado. Esa parte del informe puede estar incompleta: subí el «Tope de salida» en Configuración › Agente o usá un modelo que admita respuestas más largas.'
    )
  })

  it('says that archive batches retried smaller were still read', () => {
    expect(
      describeRoleWarnings([
        {
          role: 'asistente_archivo',
          error: 'el lote de archivo necesitó reintentos con lotes más chicos',
          times: 2,
        },
      ])
    ).toEqual([
      {
        message:
          'Lectura de documentos: algunos lotes se tuvieron que leer en partes más chicas (2 veces). Los documentos se leyeron igual; si pasa seguido, probá con un modelo más capaz.',
        detail: null,
      },
    ])
  })

  it('keeps any other engine message, naming the step instead of its internal id', () => {
    expect(
      describeRoleWarnings([
        {
          role: 'investigador_principal',
          error: 'plan incompleto; se usa la pregunta como consulta',
        },
        {
          role: 'asistente_bibliografia',
          error: 'asistente_bibliografia pidió herramientas fuera de su contrato',
          times: 2,
        },
        { role: 'recuperacion', error: 'recuperación solo léxica' },
      ])
    ).toEqual([
      {
        message:
          'Planificación de la investigación: plan incompleto; se usa la pregunta como consulta.',
        detail: null,
      },
      {
        message:
          'Lectura de la bibliografía: lectura de la bibliografía pidió herramientas fuera de su contrato (2 veces).',
        detail: null,
      },
      { message: 'Búsqueda de fragmentos: recuperación solo léxica.', detail: null },
    ])
  })

  it('never shows the raw id of a role it does not know', () => {
    const [line] = describeRoleWarnings([{ role: 'rol_nuevo', error: 'algo falló' }])

    expect(line).toEqual({ message: 'Un paso de la investigación: algo falló.', detail: null })
    expect(describeRoleWarnings([{ error: 'sin rol' }])[0]!.message).toBe(
      'Un paso de la investigación: sin rol.'
    )
  })

  it('drops warnings without an error and repeated identical ones', () => {
    const warning = { role: 'redactor', error: 'se usó el borrador anterior' }

    expect(
      describeRoleWarnings([
        warning,
        { role: 'redactor', error: '  ' },
        { role: 'redactor' },
        warning,
      ])
    ).toEqual([{ message: 'Redacción del informe: se usó el borrador anterior.', detail: null }])
  })

  it('speaks English when the interface is in English', () => {
    locale.set('en')
    const error = 'Salida inválida de asistente_archivo: la salida no contiene JSON recuperable'

    expect(describeRoleWarnings([{ role: 'asistente_archivo', error, times: 2 }])).toEqual([
      {
        message:
          'Document reading: the model returned an answer in a format the program cannot read (2 times), so that step went on without its result. That part of the report may be incomplete; try a more capable model.',
        detail: error,
      },
    ])
  })
})
