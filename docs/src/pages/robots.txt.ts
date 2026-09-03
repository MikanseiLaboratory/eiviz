import type { APIRoute } from 'astro';

export const GET: APIRoute = ({ site }) => {
	const origin = site ?? new URL('https://mikanseilaboratory.github.io');
	const sitemapURL = new URL(`${import.meta.env.BASE_URL}sitemap-index.xml`, origin);
	const body = `User-agent: *
Allow: /

Sitemap: ${sitemapURL.href}
`;
	return new Response(body, {
		headers: {
			'Content-Type': 'text/plain; charset=utf-8',
		},
	});
};
