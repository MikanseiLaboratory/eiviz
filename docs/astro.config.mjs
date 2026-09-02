// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

// https://astro.build/config
// GitHub Pages: https://mikanseilaboratory.github.io/eiviz/
export default defineConfig({
	site: 'https://mikanseilaboratory.github.io',
	base: '/eiviz/',
	integrations: [
		starlight({
			title: 'eiviz',
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
						'introduction/requirements',
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
	],
});
