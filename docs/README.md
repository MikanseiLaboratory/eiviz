# eiviz docs

Starlight starter (`npm create astro@latest -- --template starlight`).

Public URL: [https://mikanseilaboratory.github.io/eiviz/](https://mikanseilaboratory.github.io/eiviz/)

After each production deploy, `sitemap-index.xml` and `robots.txt` are published at the site root (`/eiviz/`). Submit this sitemap in [Google Search Console](https://search.google.com/search-console) for the `https://mikanseilaboratory.github.io/eiviz/` prefix:

- https://mikanseilaboratory.github.io/eiviz/sitemap-index.xml

Search Console property verification has to be done by a GitHub org owner (URL-prefix or HTML-file). The build cannot register the property by itself.

Japanese is the source of truth (`src/content/docs/ja/`). English lives in `src/content/docs/en/` with the same relative paths.

Requires Node.js 22.12 or later.

## 🚀 Project Structure

```
.
├── public/
├── src/
│   ├── assets/
│   ├── content/
│   │   └── docs/
│   │       ├── ja/
│   │       └── en/
│   └── content.config.ts
├── astro.config.mjs
├── package.json
└── tsconfig.json
```

Add pages as Markdown in both `ja/` and `en/`, then add the slug to `sidebar` in `astro.config.mjs`.

## 🧞 Commands

All commands are run from `docs/`, from a terminal:

| Command                   | Action                                           |
| :------------------------ | :----------------------------------------------- |
| `npm install`             | Installs dependencies                            |
| `npm run dev`             | Starts local dev server at `localhost:4321/eiviz/` |
| `npm run build`           | Build your production site to `./dist/`          |
| `npm run preview`         | Preview your build locally, before deploying     |
| `npm run astro ...`       | Run CLI commands like `astro add`, `astro check` |
| `npm run astro -- --help` | Get help using the Astro CLI                     |
