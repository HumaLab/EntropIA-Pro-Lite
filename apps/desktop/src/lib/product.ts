/**
 * Build-time product-identity constants for the unified Lite/Pro build.
 *
 * This module is a SEPARATE axis from `$lib/capabilities`: it expresses PRODUCT
 * IDENTITY (name, badge, repo URL), not capability. It reads the same
 * `VITE_LOCAL_ML` define()'d literal so each constant tree-shakes to a single
 * string per build, but brand must never become a `{#if LOCAL_ML}` template
 * branch — keep it here so the two concerns stay decoupled.
 */
const isPro = import.meta.env.VITE_LOCAL_ML === '1'

export const APP_VERSION = import.meta.env.VITE_APP_VERSION
export const PRODUCT_NAME = import.meta.env.VITE_PRODUCT_NAME
export const PRODUCT_NAME_BADGE = `${PRODUCT_NAME} β`
export const GITHUB_REPO_URL = isPro
  ? 'https://github.com/HumaLab/EntropIA-Pro-Lite'
  : 'https://github.com/HumaLab/EntropIA-Pro-Lite'
