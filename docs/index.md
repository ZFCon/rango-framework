# Rango documentation

Rango is a batteries-included web framework for Rust: an ORM with migrations, a template language, forms, an admin
site, authentication, a REST framework with OpenAPI docs, and the pieces around them (cache, mail, files, i18n).

On crates.io the package is `rangoframework`; in code the library is `rango`.

## Start here

- [Install](intro/install.md) — the CLI, a new project, adding Rango to an existing crate.
- Tutorial — build a polls app from an empty directory:
  1. [Project and first view](tutorial/01-project.md)
  2. [Models and the database](tutorial/02-models.md)
  3. [Views and templates](tutorial/03-views-templates.md)
  4. [Forms and generic views](tutorial/04-forms-generic-views.md)
  5. [Testing](tutorial/05-testing.md)
  6. [Static files](tutorial/06-static-files.md)
  7. [The admin](tutorial/07-admin.md)
  8. [A REST API](tutorial/08-rest-api.md)
- [Guide](guide.md) — module map, rules of thumb and design notes on one page.
- [Feature checklist](features.md) — what is implemented, partial, or not started.

## The database layer

| Topic | Reference |
|---|---|
| [Models](topics/db/models.md) | [Model fields](ref/model-fields.md) |
| [Queries](topics/db/queries.md) | [QuerySet API](ref/queryset-api.md) |
| [Relations](topics/db/relations.md) | |
| [Aggregation and expressions](topics/db/aggregation.md) | |
| [Transactions](topics/db/transactions.md) | |
| [Migrations](topics/db/migrations.md) | |
| [Databases](topics/db/databases.md) | |
| [Serialization and fixtures](topics/serialization.md) | |

## Requests, URLs and views

| Topic | Reference |
|---|---|
| [URLs](topics/http/urls.md) | |
| [Views](topics/http/views.md) | |
| [Requests and responses](topics/http/requests-responses.md) | |
| [Generic views](topics/http/generic-views.md) | |
| [Shortcuts](topics/http/shortcuts.md) | |
| [Middleware](topics/http/middleware.md) | [Built-in middleware](ref/middleware.md) |
| [File uploads](topics/http/file-uploads.md) | |
| [Pagination](topics/pagination.md) | |

## Templates

| Topic | Reference |
|---|---|
| [The template language](topics/templates/language.md) | [Built-in tags](ref/template-tags.md) |
| [The engine: settings, loaders, context](topics/templates/engine.md) | [Built-in filters](ref/template-filters.md) |
| [Custom tags and filters](topics/templates/custom-tags.md) | |

## Forms

| Topic | Reference |
|---|---|
| [Working with forms](topics/forms/forms.md) | [Form fields](ref/form-fields.md) |
| [Validation](topics/forms/validation.md) | [Widgets](ref/widgets.md) |
| [Rendering](topics/forms/rendering.md) | |
| [Model forms](topics/forms/modelforms.md) | |
| [Formsets](topics/forms/formsets.md) | |
| [Validators](topics/validators.md) | |

## Users, sessions and the admin

- [Authentication overview](topics/auth/overview.md)
- [Passwords](topics/auth/passwords.md)
- [Auth views and forms](topics/auth/views-forms.md)
- [Custom user models](topics/auth/custom-user.md)
- [Sessions](topics/sessions.md)
- [Messages](topics/messages.md)
- [The admin site](topics/admin.md)

## REST APIs

- [Quickstart](topics/rest/quickstart.md)
- [Serializers](topics/rest/serializers.md)
- [Viewsets and routers](topics/rest/viewsets-routers.md)
- [API views, requests, responses, errors](topics/rest/api-views.md)
- [Authentication, permissions, throttling](topics/rest/auth-permissions.md)
- [Pagination and filtering](topics/rest/pagination-filtering.md)
- [OpenAPI schema and docs](topics/rest/openapi.md)
- [CORS](topics/cors.md)

## Project and core

| Topic | Reference |
|---|---|
| [Settings](topics/settings.md) | [All settings](ref/settings.md) |
| [Management commands](topics/commands.md) | [Built-in commands](ref/commands.md) |
| [Applications](topics/apps.md) | |
| [System checks](topics/checks.md) | |
| [Signals](topics/signals.md) | |
| [Testing](topics/testing.md) | |

## Batteries

- [Cache](topics/cache.md)
- [Email](topics/mail.md)
- [Files and storage](topics/files.md)
- [Static files](topics/static-files.md)
- [Signing](topics/signing.md)
- [Translation (i18n)](topics/i18n.md)
- [Formats and time zones (l10n)](topics/l10n.md)
- [Sitemaps](topics/sitemaps.md)
- [Feeds (RSS, Atom)](topics/feeds.md)
- [Sites, flat pages, redirects](topics/sites-flatpages-redirects.md)
- [Content types](topics/contenttypes.md)
- [Utilities](topics/utils.md)

## Going to production

- [Security](topics/security.md)
- [Deployment](topics/deployment.md)

## Project

- [Colour scheme and logo](design.md)
- Source: <https://github.com/ZFCon/rango-framework>
