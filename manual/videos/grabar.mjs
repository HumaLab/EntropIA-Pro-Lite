/**
 * Graba los videos del manual (T-40) sobre la app de escritorio.
 * ---------------------------------------------------------------------
 * La app de prueba se abre con el control remoto de WebView2 encendido:
 *
 *   $env:ENTROPIA_DEV_PROFILE = "videos"
 *   $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=9222"
 *   apps\desktop\src-tauri\target\debug\entropia-pro-desktop.exe
 *
 * y este script le habla por el puerto 9222 (Chrome DevTools Protocol): le
 * agrega un cursor dibujado y un cartel, hace clic de verdad donde apunta el
 * cursor y guarda los cuadros que manda la pantalla. ffmpeg los arma en mp4.
 *
 * Uso:  node manual/videos/grabar.mjs            (todos)
 *       node manual/videos/grabar.mjs temas      (uno; nombres en GUIONES)
 *
 * Los guiones están en el vault: «Manual en video — guiones de las funciones
 * nuevas (2026-10-06)». Salen en manual/videos/salida/*.mp4.
 */
import { execFileSync } from 'node:child_process'
import { mkdirSync, rmSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const FFMPEG =
  process.env.FFMPEG_BIN || 'C:/laragon/www/antares/node_modules/ffmpeg-static/ffmpeg.exe'
const PUERTO = process.env.CDP_PORT || '9222'
const ANCHO = 1280
const ALTO = 800
const SALIDA = join(dirname(fileURLToPath(import.meta.url)), 'salida')

/* -------------------------------------------------- conexión remota --- */
async function conectar() {
  const lista = await (await fetch(`http://127.0.0.1:${PUERTO}/json`)).json()
  const pagina = lista.find((p) => p.type === 'page')
  if (!pagina) throw new Error('la app no expone ninguna página en el puerto ' + PUERTO)
  const ws = new WebSocket(pagina.webSocketDebuggerUrl)
  await new Promise((ok, mal) => ((ws.onopen = ok), (ws.onerror = mal)))
  let id = 0
  const pendientes = new Map()
  const oyentes = new Map()
  ws.onmessage = (evento) => {
    const m = JSON.parse(evento.data)
    if (m.id && pendientes.has(m.id)) {
      pendientes.get(m.id)(m)
      pendientes.delete(m.id)
    } else if (m.method && oyentes.has(m.method)) oyentes.get(m.method)(m.params)
  }
  const enviar = (method, params = {}) =>
    new Promise((ok) => {
      const i = ++id
      pendientes.set(i, ok)
      ws.send(JSON.stringify({ id: i, method, params }))
    })
  return { ws, enviar, oir: (method, fn) => oyentes.set(method, fn) }
}

const esperar = (ms) => new Promise((ok) => setTimeout(ok, ms))

/* ------------------------------------------------- superposiciones --- */
const SUPERPOSICION = `(() => {
  if (document.getElementById('demo-cursor')) return;
  const c = document.createElement('div');
  c.id = 'demo-cursor';
  c.style.cssText = 'position:fixed;left:640px;top:400px;width:26px;height:26px;z-index:2147483647;' +
    'pointer-events:none;transition:left .55s cubic-bezier(.22,1,.36,1),top .55s cubic-bezier(.22,1,.36,1);' +
    'filter:drop-shadow(0 2px 6px rgba(0,0,0,.65))';
  c.innerHTML = '<svg width="26" height="26" viewBox="0 0 24 24"><path d="M4 2 4 20 9 15.5 12.5 22 15 20.8 11.6 14.4 18 14Z" fill="#fff" stroke="#1b1b1b" stroke-width="1.3"/></svg>';
  const k = document.createElement('div');
  k.id = 'demo-cartel';
  k.style.cssText = 'position:fixed;left:50%;bottom:40px;transform:translateX(-50%);z-index:2147483646;' +
    'pointer-events:none;background:rgba(10,12,18,.94);color:#eef1f7;font:600 18px/1.4 system-ui,sans-serif;' +
    'padding:.75rem 1.4rem;border-radius:999px;border:1px solid rgba(154,164,199,.6);max-width:82%;' +
    'text-align:center;opacity:0;transition:opacity .35s;box-shadow:0 8px 30px rgba(0,0,0,.5)';
  const s = document.createElement('style');
  s.textContent = '@keyframes demoPulse{from{transform:scale(.4);opacity:.95}to{transform:scale(1.5);opacity:0}}';
  document.head.appendChild(s);
  document.body.appendChild(c);
  document.body.appendChild(k);
})()`

/* ------------------------------------------------------- un guion --- */
async function grabar(nombre, guion) {
  const { ws, enviar, oir } = await conectar()
  const js = async (expresion) => {
    const r = await enviar('Runtime.evaluate', {
      expression: `(async () => { ${expresion} })()`,
      awaitPromise: true,
      returnByValue: true,
    })
    if (r.result?.exceptionDetails) throw new Error(JSON.stringify(r.result.exceptionDetails))
    return r.result?.result?.value
  }

  const dir = join(SALIDA, `tmp-${nombre}`)
  rmSync(dir, { recursive: true, force: true })
  mkdirSync(dir, { recursive: true })
  const cuadros = []
  oir('Page.screencastFrame', (p) => {
    const archivo = join(dir, `${String(cuadros.length).padStart(5, '0')}.jpg`)
    writeFileSync(archivo, Buffer.from(p.data, 'base64'))
    cuadros.push({ archivo, t: p.metadata.timestamp })
    enviar('Page.screencastFrameAck', { sessionId: p.sessionId })
  })

  await enviar('Emulation.setDeviceMetricsOverride', {
    width: ANCHO,
    height: ALTO,
    deviceScaleFactor: 1,
    mobile: false,
  })
  await js(SUPERPOSICION)
  await enviar('Page.startScreencast', { format: 'jpeg', quality: 85, maxWidth: ANCHO, maxHeight: ALTO })

  /** Centro del primer elemento visible que coincide (selector CSS o texto). */
  const ubicar = async ({ css, texto, n = 0, dentro = 'button,a,[role=tab],[role=menuitem],[role=menuitemradio],label,option,summary' }) =>
    js(`
      const visibles = (lista) => [...lista].filter((el) => { const r = el.getBoundingClientRect(); return r.width > 0 && r.height > 0 });
      const t = ${JSON.stringify(texto ?? null)};
      const el = ${JSON.stringify(css ?? null)}
        ? visibles(document.querySelectorAll(${JSON.stringify(css ?? '')}))[${n}]
        : visibles(document.querySelectorAll(${JSON.stringify(dentro)})).filter((e) =>
            (e.getAttribute('aria-label') || e.textContent || '').trim().replace(/\\s+/g, ' ').includes(t))[${n}]
      if (!el) return null
      el.scrollIntoView({ block: 'center' })
      const r = el.getBoundingClientRect()
      return { x: r.x + r.width / 2, y: r.y + r.height / 2 }
    `)

  const t = {
    js,
    async cartel(texto, espera = 1800) {
      await js(`const k = document.getElementById('demo-cartel'); if (k) { k.textContent = ${JSON.stringify(texto)}; k.style.opacity = ${texto ? 1 : 0} }`)
      await esperar(espera)
    },
    async mover(blanco) {
      let punto = null
      for (let i = 0; i < 30 && !punto; i++) {
        punto = await ubicar(blanco)
        if (!punto) await esperar(300)
      }
      if (!punto) throw new Error('no encontré ' + JSON.stringify(blanco))
      await js(`const c = document.getElementById('demo-cursor'); c.style.left = '${punto.x - 3}px'; c.style.top = '${punto.y - 2}px'`)
      await esperar(800)
      return punto
    },
    async clic(blanco) {
      const { x, y } = await t.mover(blanco)
      await js(`const c = document.getElementById('demo-cursor'); const o = document.createElement('div');
        o.style.cssText = 'position:fixed;width:44px;height:44px;border-radius:50%;border:2.5px solid #9aa4c7;z-index:2147483645;pointer-events:none;left:' + (parseFloat(c.style.left) - 10) + 'px;top:' + (parseFloat(c.style.top) - 10) + 'px;animation:demoPulse .5s ease-out forwards';
        document.body.appendChild(o); setTimeout(() => o.remove(), 550)`)
      await esperar(250)
      for (const type of ['mousePressed', 'mouseReleased'])
        await enviar('Input.dispatchMouseEvent', { type, x, y, button: 'left', clickCount: 1 })
      await esperar(700)
    },
    async tipear(blanco, texto) {
      await t.clic(blanco)
      for (const letra of texto) {
        await enviar('Input.insertText', { text: letra })
        await esperar(55)
      }
      await esperar(400)
    },
    esperar,
    /** Espera hasta que el texto aparezca en pantalla (o pasen `ms`). */
    async esperarTexto(texto, ms = 20000) {
      for (let pasado = 0; pasado < ms; pasado += 400) {
        if (await js(`return document.body.innerText.includes(${JSON.stringify(texto)})`)) return
        await esperar(400)
      }
    },
    /** Marca la casilla con ese texto solo si no está marcada. */
    async marcar(texto) {
      const marcada = await js(`const l = [...document.querySelectorAll('label')].find((e) => e.textContent.includes(${JSON.stringify(texto)})); return !!l?.querySelector('input')?.checked`)
      if (marcada) await t.mover({ texto, dentro: 'label' })
      else await t.clic({ texto, dentro: 'label' })
    },
    /** Deja la pestaña de lotes sin borrador ni casillas de un video anterior. */
    async limpiarLotes() {
      await js(`const b = [...document.querySelectorAll('button')].find((e) => e.textContent.trim() === 'Descartar borrador'); b?.click()`)
      await esperar(800)
    },
  }

  try {
    await guion(t)
    await t.cartel('', 600)
  } finally {
    await enviar('Page.stopScreencast')
    await enviar('Emulation.clearDeviceMetricsOverride')
    ws.close()
  }

  // Cada cuadro dura hasta el siguiente: así el video respeta los tiempos reales.
  const lista = cuadros
    .map((c, i) => {
      const dura = i + 1 < cuadros.length ? cuadros[i + 1].t - c.t : 1
      return `file '${c.archivo.replaceAll('\\', '/')}'\nduration ${dura.toFixed(3)}`
    })
    .join('\n')
  const concat = join(dir, 'lista.txt')
  writeFileSync(concat, lista + `\nfile '${cuadros.at(-1).archivo.replaceAll('\\', '/')}'\n`)
  const mp4 = join(SALIDA, `${nombre}.mp4`)
  execFileSync(FFMPEG, ['-y', '-loglevel', 'error', '-f', 'concat', '-safe', '0', '-i', concat,
    '-vf', `fps=25,scale=${ANCHO}:${ALTO}:force_original_aspect_ratio=decrease,pad=${ANCHO}:${ALTO}:(ow-iw)/2:(oh-ih)/2`,
    '-c:v', 'libx264', '-crf', '20', '-pix_fmt', 'yuv420p', '-movflags', '+faststart', mp4])
  rmSync(dir, { recursive: true, force: true })
  console.log('listo', mp4, `(${cuadros.length} cuadros)`)
}

/* ------------------------------------------------------- los guiones --- */
const irA = async (t, nombre) => t.clic({ texto: nombre })

const GUIONES = {
  async ejemplo(t) {
    await t.cartel('La primera vez, EntropIA ofrece documentos de ejemplo para probar.')
    await t.clic({ texto: 'Probar con documentos de ejemplo' })
    await t.cartel('Crea una colección ficticia con nueve documentos que ya traen su texto.', 4000)
    await t.cartel('Ya se pueden buscar, analizar y procesar por lote.', 2500)
  },

  async lotes(t) {
    await t.clic({ texto: 'Abrir configuración' })
    await t.clic({ texto: 'Lotes', dentro: '[role=tab]' })
    await t.limpiarLotes()
    await t.cartel('Para sacar personas, lugares y relaciones de muchos documentos a la vez, entrá a Lotes.')
    await t.marcar('Ejemplo — Archivo Bristol')
    await t.cartel('Elegí la colección.')
    await t.marcar('Entidades')
    await t.cartel('Entidades busca personas, lugares y organizaciones.')
    await t.marcar('Tripletes')
    await t.cartel('Tripletes saca relaciones: quién hizo qué.')
    await t.clic({ texto: 'Analizar selección' })
    await t.esperarTexto('pasan por la búsqueda de entidades')
    await t.mover({ css: '.batch-tab__draft' })
    await t.cartel('Antes de empezar te dice cuántos documentos va a procesar. En Lite cada uno es una consulta paga.', 5000)
  },

  async esquema(t) {
    await t.clic({ texto: 'Abrir configuración' })
    await t.clic({ texto: 'Lotes', dentro: '[role=tab]' })
    await t.limpiarLotes()
    await t.cartel('¿Necesitás otros datos? Armá tu propio esquema.')
    await t.clic({ texto: 'Nuevo esquema' })
    const campo = (n) => ({ css: '.schema-panel__editor input:not([type=checkbox])', n })
    await t.tipear(campo(0), 'Movimientos del puerto')
    await t.cartel('Un campo por dato. En "Qué es" le explicás al modelo qué buscar.', 800)
    await t.tipear(campo(2), 'barco')
    await t.tipear(campo(3), 'nombre de la embarcación')
    await t.clic({ texto: 'Agregar campo' })
    await t.tipear(campo(4), 'carga')
    await t.tipear(campo(5), 'mercaderías que transporta')
    await t.clic({ texto: 'Puede repetirse', dentro: 'label', n: 1 })
    await t.cartel('Un barco puede traer muchas cargas: marcalo para que vengan todas.', 2200)
    await t.clic({ texto: 'Agregar campo' })
    await t.tipear(campo(6), 'destino')
    await t.tipear(campo(7), 'puerto al que se dirige')
    await t.clic({ texto: 'Guardar esquema' })
    await t.cartel('Guardalo y queda elegido para el próximo lote.', 2200)
    await t.marcar('Ejemplo — Archivo Bristol')
    await t.clic({ texto: 'Analizar selección' })
    await t.esperarTexto('pasan por el esquema propio')
    await t.cartel('Elegís la colección, analizás y lo corrés como cualquier lote.', 4000)
  },

  async temas(t) {
    await t.clic({ texto: 'Abrir configuración' })
    await t.clic({ texto: 'Apariencia', dentro: '[role=tab]' })
    await t.cartel('EntropIA tiene siete temas.')
    for (const [tema, frase] of [
      ['Papel', 'Papel imita el papel de archivo: cómodo para leer durante horas.'],
      ['Bosque', 'Bosque, oscuro con verde.'],
      ['Vibrante', 'Vibrante, el más llamativo.'],
      ['Oscuro', 'Y los de siempre.'],
    ]) {
      await t.clic({ css: '[aria-labelledby*="appearance-theme-label"]' })
      await t.clic({ texto: tema, dentro: '[role=menuitemradio]' })
      await t.cartel(frase, 2200)
    }
  },
}

/* ------------------------------------------------------------ main --- */
mkdirSync(SALIDA, { recursive: true })
const pedidos = process.argv.slice(2)
for (const [nombre, guion] of Object.entries(GUIONES)) {
  if (pedidos.length && !pedidos.includes(nombre)) continue
  try {
    await grabar(nombre, guion)
  } catch (error) {
    console.error('falló', nombre, error.message)
  }
}
