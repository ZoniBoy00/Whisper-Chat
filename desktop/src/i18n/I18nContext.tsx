import { useCallback, useContext, useEffect, useMemo, useState } from "react";
import { createContext } from "react";
import type { ReactNode } from "react";
import { getSettings, updateSettings } from "../lib/relay";
import { translations } from "./translations";
import type { Language, TFunction, TranslationKey, TranslationParams } from "./types";

interface I18nContextValue {
  /** The active UI language; "en" until a persisted choice loads. */
  language: Language;
  /** Switch the UI language and persist the choice. */
  setLanguage: (language: Language) => void;
  /** Translate a key, interpolating `{param}` placeholders. */
  t: TFunction;
}

/** Keys that exist in the persisted settings store as a language value. */
const VALID_LANGUAGES: readonly Language[] = ["en", "fi"];

function isLanguage(value: unknown): value is Language {
  return VALID_LANGUAGES.includes(value as Language);
}

/** Replace `{name}` placeholders in a template with the matching params. */
function interpolate(template: string, params?: TranslationParams): string {
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (match, name: string) =>
    name in params ? String(params[name]) : match
  );
}

const I18nContext = createContext<I18nContextValue | null>(null);

/**
 * Lightweight, dependency-free i18n provider. Loads the persisted language
 * from the settings store on mount and applies it to the whole tree via
 * `useI18n`. Changing the language re-renders every consumer and persists the
 * choice best-effort.
 */
export function I18nProvider({ children }: { children: ReactNode }) {
  const [language, setLanguageState] = useState<Language>("en");

  useEffect(() => {
    let cancelled = false;
    void getSettings()
      .then((settings) => {
        if (cancelled) return;
        if (isLanguage(settings.language)) setLanguageState(settings.language);
      })
      .catch(() => {
        // Settings are best-effort; English is the default.
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const setLanguage = useCallback((next: Language) => {
    setLanguageState(next);
    void updateSettings({ language: next }).catch(() => {
      // Persistence only affects the next launch; the UI language applies
      // in memory immediately.
    });
  }, []);

  const t = useCallback<TFunction>(
    (key: TranslationKey, params?: TranslationParams) => {
      const dict = translations[language] ?? translations.en;
      const value = dict[key] ?? translations.en[key];
      if (typeof value === "function") return value(params ?? {});
      return interpolate(value, params);
    },
    [language]
  );

  const value = useMemo<I18nContextValue>(
    () => ({ language, setLanguage, t }),
    [language, setLanguage, t]
  );

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

/** Access the active language and the `t` translation function. */
export function useI18n(): I18nContextValue {
  const context = useContext(I18nContext);
  if (!context) {
    throw new Error("useI18n must be used within an I18nProvider");
  }
  return context;
}
