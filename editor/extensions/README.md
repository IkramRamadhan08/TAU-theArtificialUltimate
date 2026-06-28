# Tau Extensions

This directory contains extensions for Tau that are largely maintained by the Tau team. They currently live in the Tau repository for ease of maintenance.

If you are looking for the Tau extension registry, see the [`tau-industries/extensions`](https://github.com/tau-industries/extensions) repo.

## Structure

Currently, Tau includes support for a number of languages without requiring installing an extension. Those languages can be found under [`crates/languages/src`](https://github.com/tau-industries/tau/tree/main/crates/languages/src).

Support for all other languages is done via extensions. This directory ([extensions/](https://github.com/tau-industries/tau/tree/main/extensions/)) contains some of the officially maintained extensions. These extensions use the same [zed_extension_api](https://docs.rs/zed_extension_api/latest/zed_extension_api/) available to all [Tau Extensions](https://tau.dev/extensions) for providing [language servers](https://tau.dev/docs/extensions/languages#language-servers), [tree-sitter grammars](https://tau.dev/docs/extensions/languages#grammar) and [tree-sitter queries](https://tau.dev/docs/extensions/languages#tree-sitter-queries).

You can find the other officially maintained extensions in the [tau-extensions organization](https://github.com/tau-extensions).

## Dev Extensions

See the docs for [Developing an Extension Locally](https://tau.dev/docs/extensions/developing-extensions#developing-an-extension-locally) for how to work with one of these extensions.
