// @ts-check
import { defineConfig } from 'astro/config';
import sitemap from '@astrojs/sitemap';
import starlight from '@astrojs/starlight';
import mermaid from 'astro-mermaid';

const SITE = 'https://mikanseilaboratory.github.io';
const BASE = '/eiviz/';

async function latestReleaseTag() {
	if (process.env.PUBLIC_EIVIZ_VERSION) {
		return process.env.PUBLIC_EIVIZ_VERSION;
	}
	try {
		const headers = new Headers({
			Accept: 'application/vnd.github+json',
			'User-Agent': 'eiviz-docs',
		});
		if (process.env.GITHUB_TOKEN) {
			headers.set('Authorization', `Bearer ${process.env.GITHUB_TOKEN}`);
		}
		const res = await fetch(
			'https://api.github.com/repos/MikanseiLaboratory/eiviz/releases?per_page=1',
			{ headers },
		);
		if (!res.ok) {
			return '';
		}
		const releases = await res.json();
		return typeof releases?.[0]?.tag_name === 'string' ? releases[0].tag_name : '';
	} catch {
		return '';
	}
}

const version = await latestReleaseTag();
if (version) {
	process.env.PUBLIC_EIVIZ_VERSION = version;
}

// https://astro.build/config
// GitHub Pages: https://mikanseilaboratory.github.io/eiviz/
export default defineConfig({
	site: SITE,
	base: BASE,
	integrations: [
		mermaid({ autoTheme: true }),
		starlight({
			title: 'eiviz',
			description: 'A cross-platform vision mixer with unlimited M/E.',
			logo: {
				light: './src/assets/logo-light.png',
				dark: './src/assets/logo-dark.png',
				alt: 'Mikansei Laboratory',
			},
			favicon: '/favicon.png',
			components: {
				Footer: './src/overrides/Footer.astro',
			},
			head: [
				{
					tag: 'meta',
					attrs: {
						property: 'og:image',
						content: `${SITE}${BASE}og.png`,
					},
				},
				{
					tag: 'meta',
					attrs: {
						property: 'og:image:width',
						content: '1200',
					},
				},
				{
					tag: 'meta',
					attrs: {
						property: 'og:image:height',
						content: '630',
					},
				},
				{
					tag: 'meta',
					attrs: {
						property: 'og:image:alt',
						content: 'eiviz',
					},
				},
				{
					tag: 'meta',
					attrs: {
						property: 'og:image:type',
						content: 'image/png',
					},
				},
				{
					tag: 'meta',
					attrs: {
						name: 'twitter:image',
						content: `${SITE}${BASE}og.png`,
					},
				},
				{
					tag: 'link',
					attrs: {
						rel: 'sitemap',
						href: `${BASE}sitemap-index.xml`,
					},
				},
				{
					tag: 'link',
					attrs: {
						rel: 'icon',
						type: 'image/png',
						href: '/eiviz/favicon.png',
						media: '(prefers-color-scheme: light)',
					},
				},
				{
					tag: 'link',
					attrs: {
						rel: 'icon',
						type: 'image/png',
						href: '/eiviz/favicon-dark.png',
						media: '(prefers-color-scheme: dark)',
					},
				},
			],
			defaultLocale: 'ja',
			locales: {
				ja: { label: '日本語' },
				en: { label: 'English' },
			},
			social: [
				{ icon: 'github', label: 'GitHub', href: 'https://github.com/MikanseiLaboratory/eiviz' },
			],
			editLink: {
				baseUrl: 'https://github.com/MikanseiLaboratory/eiviz/edit/main/docs/',
			},
			sidebar: [
				{
					label: 'Introduction',
					translations: { ja: 'はじめに' },
					items: [
						'introduction/about',
						'introduction/mikansei-laboratory',
						'introduction/requirements',
						'introduction/settings',
						'introduction/architecture',
					],
				},
				{
					label: 'Concepts',
					translations: { ja: 'eiviz上の概念' },
					items: [
						'concepts/inputs',
						'concepts/scenes',
						'concepts/mixing-unit',
						'concepts/audio-auxs',
						'concepts/outputs',
						'concepts/multiviews',
						'concepts/overlays',
					],
				},
				{
					label: 'Features',
					translations: { ja: '各種機能' },
					items: [
						{
							label: 'Inputs',
							translations: { ja: '入力系' },
							items: [
								'features/inputs/uvc',
								'features/inputs/ndi-omt',
								'features/inputs/media',
								'features/inputs/colour',
							],
						},
						'features/compositing',
						{
							label: 'Outputs',
							translations: { ja: '出力系' },
							items: [
								'features/outputs/ndi-omt',
								'features/outputs/decklink',
								'features/outputs/audio',
							],
						},
						'features/vision-mixing',
					],
				},
				{
					label: 'Developers',
					translations: { ja: '開発者向け情報' },
					items: [
						{
							label: 'APIs',
							translations: { ja: '現在実装しているAPI' },
							items: [
								'developers/compatibility',
								'developers/api',
								'developers/function-reference',
							],
						},
					],
				},
			],
		}),
		sitemap({
			i18n: {
				defaultLocale: 'ja',
				locales: {
					ja: 'ja',
					en: 'en',
				},
			},
		}),
	],
});
