import {createRequire} from 'node:module';
import {themes as prismThemes} from 'prism-react-renderer';

const require = createRequire(import.meta.url);

// Enable Algolia DocSearch after approval by setting these env vars.
// Without them, the docs keep the existing local search plugin as a fallback.
const docSearchAppId = process.env.DOCSEARCH_APP_ID ?? process.env.ALGOLIA_APP_ID;
const docSearchApiKey = process.env.DOCSEARCH_API_KEY ?? process.env.ALGOLIA_API_KEY;
const docSearchIndexName = process.env.DOCSEARCH_INDEX_NAME ?? process.env.ALGOLIA_INDEX_NAME;
const siteUrl = process.env.DOCS_URL ?? 'https://ciallothu.github.io';
const siteBaseUrl = process.env.DOCS_BASE_URL ?? '/oxidns-next/';

const algoliaConfig = docSearchAppId && docSearchApiKey && docSearchIndexName
  ? {
      appId: docSearchAppId,
      apiKey: docSearchApiKey,
      indexName: docSearchIndexName,
      contextualSearch: true,
      searchParameters: {},
    }
  : undefined;

/** @type {import('@docusaurus/types').Config} */
const config = {
  title: 'OxiDNS Next',
  tagline: 'A Rust-powered DNS engine inspired by MosDNS, designed for performance and complete configurability.',
  favicon: 'img/logo-next-light.png',

  scripts: [
    {
      src: `${siteBaseUrl}js/theme-favicon.js`,
      defer: true,
    },
  ],

  future: {
    v4: true,
  },

  url: siteUrl,
  baseUrl: siteBaseUrl,

  organizationName: 'ciallothu',
  projectName: 'oxidns-next',

  onBrokenLinks: 'throw',

  i18n: {
    defaultLocale: 'zh-Hans',
    locales: ['zh-Hans', 'en'],
    localeConfigs: {
      'zh-Hans': {
        label: '中文',
      },
      en: {
        label: 'English',
      },
    },
  },

  markdown: {
    mermaid: true,
    hooks: {
      onBrokenMarkdownLinks: 'throw',
    },
  },

  themes: ['@docusaurus/theme-mermaid'],

  plugins: [
    !algoliaConfig && [
      require.resolve('@easyops-cn/docusaurus-search-local'),
      {
        hashed: true,
        docsRouteBasePath: '/',
        indexDocs: true,
        indexBlog: false,
        indexPages: false,
        language: ['zh', 'en'],
        highlightSearchTermsOnTargetPage: true,
        searchBarShortcut: true,
        searchBarShortcutHint: true,
        searchResultLimits: 8,
        explicitSearchResultPath: true,
      },
    ],
  ].filter(Boolean),

  presets: [
    [
      '@docusaurus/preset-classic',
      ({
        docs: {
          path: './docs',
          routeBasePath: '/',
          sidebarPath: './sidebars.js',
          editUrl: 'https://github.com/ciallothu/oxidns-next/tree/main/docs/',
        },
        blog: false,
        theme: {
          customCss: './src/css/custom.css',
        },
      }),
    ],
  ],

  themeConfig: ({
      colorMode: {
        defaultMode: 'light',
        disableSwitch: false,
        respectPrefersColorScheme: false,
      },
      navbar: {
        title: 'OxiDNS Next',
        logo: {
          alt: 'OxiDNS Next Logo',
          src: 'img/logo-next-light.png',
          srcDark: 'img/logo-next-dark.png',
          width: 32,
          height: 32,
        },
        items: [
          {
            type: 'localeDropdown',
            position: 'right',
          },
          {
            href: 'https://github.com/ciallothu/oxidns-next',
            'aria-label': 'GitHub repository',
            className: 'header-github-link',
            position: 'right',
          },
          {
            type: 'search',
            position: 'right',
          },
        ],
      },
      footer: {
        style: 'light',
        links: [
        ],
        copyright: `Copyright © ${new Date().getFullYear()} OxiDNS Next contributors · Based on OxiDNS by Sven Shi`,
      },
      prism: {
        theme: prismThemes.oneDark,
        darkTheme: prismThemes.oneDark,
        additionalLanguages: ['shell-session', 'powershell', 'bash'],
      },
      ...(algoliaConfig ? {algolia: algoliaConfig} : {}),
    }),
};

export default config;
