/**
 * Config aparte para `generar-ejemplos-puente.ts`.
 *
 * Existe para que ese guion NO entre en la batería normal (`pnpm test`), que
 * sólo mira `src/**`. Reusa todo lo demás de la config principal (los plugins,
 * los alias, el entorno de prueba y los remedos de Tauri).
 */
import { defineConfig, mergeConfig, type UserConfig } from 'vitest/config'
import { resolve } from 'path'
import baseConfig from '../vitest.config'

const config = mergeConfig(
  baseConfig as UserConfig,
  defineConfig({
    root: resolve(__dirname, '..'),
    test: { name: 'ejemplos-puente' },
  })
) as UserConfig

// A propósito NO se mergean los patrones: `mergeConfig` los concatenaría y
// terminaría corriendo toda la batería además del guion.
config.test!.include = ['scripts/generar-ejemplos-puente.ts']

export default config
