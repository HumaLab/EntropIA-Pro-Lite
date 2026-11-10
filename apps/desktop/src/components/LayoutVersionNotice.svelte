<!--
  What the Layout tab shows instead of a layout that was computed on another
  version of the image: the layout is kept, it is just not this image's, and
  running it again replaces it.
-->
<script lang="ts">
  import type { I18nKey, I18nParams } from '$lib/i18n'
  import type { LayoutVersionVerdict } from '$lib/layout-version'
  import OtherVersionNotice from './OtherVersionNotice.svelte'

  let {
    verdict,
    busy = false,
    translate,
    onRerun,
  }: {
    verdict: Extract<LayoutVersionVerdict, { status: 'other_version' }>
    /** An OCR of this image is already running. */
    busy?: boolean
    translate: (key: I18nKey, params?: I18nParams) => string
    onRerun: () => void
  } = $props()

  // The sizes explain the mismatch only when they differ: an erase or a half
  // turn changes the image and keeps its size.
  let message = $derived.by(() => {
    const { layout, current } = verdict
    if (!current || (current.width === layout.width && current.height === layout.height)) {
      return translate('item.layoutOtherVersion')
    }
    return translate('item.layoutOtherVersionSized', {
      width: layout.width,
      height: layout.height,
      currentWidth: current.width,
      currentHeight: current.height,
    })
  })
</script>

<OtherVersionNotice
  testId="layout-version-notice"
  heading={translate('item.layoutTab')}
  {message}
  actionLabel={translate('item.layoutRerun')}
  {busy}
  busyTitle={translate('item.ocrHighBusyTitle')}
  {onRerun}
/>
