import assert from 'node:assert/strict';
import test from 'node:test';
import { ServiceTokenProvider, validateServiceTokenResponse } from '../src/service-auth.js';

const config = (issuer: string, tokenUrl: string) => ({
  issuer,
  tokenUrl,
  clientId: 'paladinscat-discord-service',
  privateKeyFile: 'missing-runtime-key.pem',
});

test('service auth rejects non-HTTPS realms and token endpoint redirects before reading a key', () => {
  assert.throws(() => new ServiceTokenProvider(config(
    'http://auth.example/realms/paladinscat',
    'http://keycloak:8080/realms/paladinscat/protocol/openid-connect/token',
  )), /HTTPS Keycloak realm/);
  assert.throws(() => new ServiceTokenProvider(config(
    'https://auth.example/realms/paladinscat',
    'https://evil.example/realms/paladinscat/protocol/openid-connect/token',
  )), /exact configured realm token endpoint/);
});

test('service token response requires exact Bearer type, bounded lifetime, and token size', () => {
  assert.deepEqual(validateServiceTokenResponse({ access_token: 'opaque', token_type: 'Bearer', expires_in: 60 }), { value: 'opaque', expiresIn: 60 });
  for (const response of [
    { access_token: 'opaque', token_type: 'bearer', expires_in: 60 },
    { access_token: 'opaque', token_type: 'Bearer', expires_in: 30 },
    { access_token: 'opaque', token_type: 'Bearer', expires_in: 301 },
    { access_token: ' opaque', token_type: 'Bearer', expires_in: 60 },
  ]) assert.throws(() => validateServiceTokenResponse(response), /invalid/);
});
