(function () {
  var scriptUrl = document.currentScript && document.currentScript.src;
  var staticRoot = scriptUrl ? new URL('../', scriptUrl) : new URL('./', window.location.origin);

  function currentTheme() {
    var theme = document.documentElement.getAttribute('data-theme');

    if (theme === 'dark' || theme === 'light') {
      return theme;
    }

    return window.matchMedia('(prefers-color-scheme: dark)').matches
      ? 'dark'
      : 'light';
  }

  function updateFavicon() {
    var logo =
      currentTheme() === 'dark' ? 'img/logo-next-dark.png' : 'img/logo-next-light.png';
    var href = new URL(logo, staticRoot).href;
    var icons = document.querySelectorAll("link[rel~='icon']");
    var icon = document.querySelector('link[data-oxidns-next-theme-icon]');

    if (icons.length === 0) {
      icon = document.createElement('link');
      icon.setAttribute('rel', 'icon');
      icon.setAttribute('data-oxidns-next-theme-icon', 'true');
      document.head.appendChild(icon);
      icons = [icon];
    }

    icons.forEach(function (entry) {
      entry.setAttribute('type', 'image/png');
      entry.setAttribute('href', href);
    });
  }

  updateFavicon();

  document.addEventListener('DOMContentLoaded', updateFavicon);

  new MutationObserver(updateFavicon).observe(document.documentElement, {
    attributes: true,
    attributeFilter: ['data-theme'],
  });

  window
    .matchMedia('(prefers-color-scheme: dark)')
    .addEventListener('change', updateFavicon);
})();
