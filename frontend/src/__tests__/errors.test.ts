import { describe, expect, it } from 'vitest';
import { errorMessage } from '../api';
import { translations } from '../translations';

const responseError = (code: unknown, message: unknown = 'backend diagnostic') => ({ response: { data: { code, message } } });

describe('FastDL API errors', () => {
  it.each(['en', 'ru'] as const)('explains the failed step and recovery in %s', (locale) => {
    const text = translations[locale];
    const trans = (key: string) => text[key as keyof typeof text];
    expect(errorMessage(responseError('CONFIGURE_FAILED'), text.save_failed, trans)).toBe(text.configure_failed);
    expect(errorMessage(responseError('GAME_CONFIG_UPDATE_FAILED'), text.save_failed, trans)).toBe(text.game_config_update_failed);
    expect(errorMessage(responseError('NODE_UNAVAILABLE'), text.save_failed, trans)).toBe(text.node_unavailable);
    expect(text.configure_failed).toContain('server.cfg');
    expect(text.configure_failed).toContain('GameAP');
  });

  it.each([
    null, undefined, new Error('secret command'),
    { response: { data: '<html>Internal server error</html>' } },
    { response: { data: { message: '{"enabled":true,"game_dir":"valve"}' } } },
    responseError('NEW_ERROR', 'secret command and server paths'),
    responseError('INVALID_INPUT', { enabled: true }),
    responseError('CONFLICT', 'unknown internal state'),
    responseError('constructor', 'prototype property'),
  ])('uses the operation fallback for an unrecognized response: %j', (error) => {
    expect(errorMessage(error, 'Не удалось сохранить настройки.', (key) => key)).toBe('Не удалось сохранить настройки.');
  });

  it('selects a known error by code without showing the backend payload', () => {
    expect(errorMessage(responseError('CONFIGURE_FAILED', '{"enabled":true}'), 'fallback', (key) => key)).toBe('configure_failed');
  });

  it.each([
    ['INVALID_INPUT', 'Listen address must be an IP address and port', 'listen_invalid'],
    ['INVALID_INPUT', 'Game directory must be a relative path inside the game server', 'game_dir_invalid'],
    ['INVALID_INPUT', 'Invalid public or download URL', 'public_url_invalid'],
    ['CONFLICT', 'Install FastDL on this node first', 'node_not_ready'],
    ['CONFLICT', 'Configure the public FastDL address first', 'public_url_required'],
    ['CONFLICT', 'Installation is already in progress', 'installation_in_progress'],
  ])('preserves useful validation for %s: %s', (code, message, key) => {
    expect(errorMessage(responseError(code, message), 'fallback', (value) => value)).toBe(key);
  });

  it.each(['en', 'ru'] as const)('explains when node settings were saved but could not be applied in %s', (locale) => {
    const text = translations[locale];
    const trans = (key: string) => text[key as keyof typeof text];
    for (const [code, suffix] of [['CONFIGURE_FAILED', 'configure_failed'], ['GAME_CONFIG_UPDATE_FAILED', 'game_config_update_failed']] as const) {
      const error = responseError(code);
      expect(errorMessage(error, text.save_failed, trans, 'node-save')).toBe(text[`node_save_${suffix}`]);
      expect(errorMessage(error, text.save_failed, trans)).toBe(text[suffix]);
    }
  });

  it('names the affected game server without relying on backend message or exposing IDs', () => {
    const trans = (key: string) => translations.ru[key as keyof typeof translations.ru];
    const error = { response: { data: { code: 'CONFIGURE_FAILED', server_name: 'Half-Life', server_id: 4, message: 'secret command' } } };
    expect(errorMessage(error, 'fallback', trans, 'node-save')).toBe(`Игровой сервер: «Half-Life». ${translations.ru.node_save_configure_failed}`);
  });

  it.each([undefined, null, {}, '', ' ', '<html>secret</html>', '{"enabled":true}', 'server\nprivate output', 'x'.repeat(201)])('ignores a malformed game server name: %j', (name) => {
    const error = { response: { data: { code: 'CONFIGURE_FAILED', server_name: name } } };
    expect(errorMessage(error, 'fallback', (key) => key, 'node-save')).toBe('node_save_configure_failed');
  });

  it('does not claim node settings were saved for validation or an unknown error', () => {
    expect(errorMessage(responseError('INVALID_INPUT', 'Invalid public or download URL'), 'fallback', (key) => key, 'node-save')).toBe('public_url_invalid');
    expect(errorMessage(responseError('UNKNOWN', 'secret'), 'fallback', (key) => key, 'node-save')).toBe('fallback');
  });

  it.each(['en', 'ru'] as const)('uses application-specific errors and readiness guidance in %s', (locale) => {
    const text = translations[locale];
    const trans = (key: string) => text[key as keyof typeof text];
    expect(errorMessage(responseError('CONFIGURE_FAILED'), text.apply_configuration_failed, trans, 'game-configure')).toBe(text.game_apply_configure_failed);
    expect(errorMessage(responseError('GAME_CONFIG_UPDATE_FAILED'), text.apply_configuration_failed, trans, 'game-configure')).toBe(text.game_apply_game_config_update_failed);
    expect(errorMessage(responseError('CONFIGURATION_NOT_READY'), text.apply_configuration_failed, trans, 'game-configure')).toBe(text.configuration_not_ready);
  });
});
