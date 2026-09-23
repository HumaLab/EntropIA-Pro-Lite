<script lang="ts">
  import { locale, t } from '$lib/i18n'

  const currentLocale = locale
</script>

<div class="home-view page-shell">
  <section class="page-header" aria-labelledby="home-title">
    <div class="page-header__content">
      <span class="page-header__eyebrow">{$currentLocale && t('home.eyebrow')}</span>
      <h1 id="home-title">{$currentLocale && t('home.title')}</h1>
    </div>
  </section>
</div>
