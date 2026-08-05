import { translations } from "./translations";

/** The UI languages the app ships. */
export type Language = "en" | "fi";

/**
 * Every key present in the English dictionary. The Finnish dictionary must
 * contain exactly the same keys — indexing the current language with a key
 * derived from English is a compile error when a Finnish string is missing.
 */
export type TranslationKey = keyof typeof translations["en"];

/** Interpolation values for a translation template (e.g. `{n}`, `{name}`). */
export type TranslationParams = Record<string, string | number>;

/** A translation value: a template string or a grammar-aware function. */
export type TranslationValue = string | ((params: TranslationParams) => string);

/** The `t` function exposed by `useI18n`. */
export type TFunction = (
  key: TranslationKey,
  params?: TranslationParams
) => string;
