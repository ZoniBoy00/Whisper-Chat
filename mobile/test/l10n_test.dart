import 'package:flutter_test/flutter_test.dart';
import 'package:whisper_mobile/src/i18n.dart';

void main() {
  group('L10n', () {
    test('English is the default language', () {
      const l = L10n('en');
      expect(l.t('app.title'), 'Whisper');
      expect(l.t('tab.chats'), 'Chats');
    });

    test('Finnish translations exist for core keys', () {
      const l = L10n('fi');
      expect(l.t('tab.chats'), 'Keskustelut');
      expect(l.t('tab.groups'), 'Ryhmät');
      expect(l.t('settings'), 'Asetukset');
      expect(l.t('online'), 'Paikalla');
    });

    test('unknown keys fall back to the key itself', () {
      const l = L10n('en');
      expect(l.t('no.such.key'), 'no.such.key');
    });
  });
}
