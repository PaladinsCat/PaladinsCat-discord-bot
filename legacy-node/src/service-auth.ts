import { createPrivateKey, createSign, randomUUID } from 'node:crypto';
import { readFileSync } from 'node:fs';

export type ServiceAuthConfig = {
  issuer: string;
  tokenUrl: string;
  clientId: string;
  privateKeyFile: string;
};

type CachedToken = { value: string; usableUntil: number };

export function validateServiceTokenResponse(body: unknown): { value: string; expiresIn: number } {
  if (body === null || typeof body !== 'object') throw new Error('OIDC service authentication response is invalid');
  const response = body as { access_token?: unknown; token_type?: unknown; expires_in?: unknown };
  if (response.token_type !== 'Bearer' || typeof response.access_token !== 'string' || response.access_token.length === 0 || response.access_token.length > 16 * 1024 || response.access_token.trim() !== response.access_token || typeof response.expires_in !== 'number' || !Number.isInteger(response.expires_in) || response.expires_in < 31 || response.expires_in > 300) throw new Error('OIDC service authentication response is invalid');
  return { value: response.access_token, expiresIn: response.expires_in };
}

export class ServiceTokenProvider {
  private cached?: CachedToken;
  private pending?: Promise<string>;
  private readonly privateKey: ReturnType<typeof createPrivateKey>;

  constructor(private readonly config: ServiceAuthConfig) {
    // Read only the runtime-mounted key; never log, serialize, or expose it to Discord/browser code.
    const issuer = new URL(config.issuer);
    const realm = issuer.pathname.match(/^\/realms\/([^/]+)$/);
    if (issuer.protocol !== 'https:' || !realm || issuer.search || issuer.hash || issuer.username || issuer.password) throw new Error('service OIDC issuer must be an HTTPS Keycloak realm URL');
    const tokenUrl = new URL(config.tokenUrl);
    const expectedPath = `${issuer.pathname}/protocol/openid-connect/token`;
    const publicMatch = tokenUrl.protocol === 'https:' && tokenUrl.hostname === issuer.hostname && tokenUrl.port === issuer.port && tokenUrl.pathname === expectedPath;
    const internalMatch = tokenUrl.protocol === 'http:' && tokenUrl.hostname === 'keycloak' && tokenUrl.port === '8080' && tokenUrl.pathname === expectedPath;
    if (tokenUrl.search || tokenUrl.hash || tokenUrl.username || tokenUrl.password || (!publicMatch && !internalMatch)) throw new Error('service OIDC token URL is not the exact configured realm token endpoint');
    const keyBytes = readFileSync(config.privateKeyFile);
    if (keyBytes.length > 32 * 1024) throw new Error('service OIDC private key exceeds 32 KiB');
    this.privateKey = createPrivateKey(keyBytes);
    if (this.privateKey.asymmetricKeyType !== 'rsa' || this.privateKey.asymmetricKeyDetails?.modulusLength == null || this.privateKey.asymmetricKeyDetails.modulusLength < 3072) throw new Error('service OIDC RSA private key must be at least 3072 bits');
  }

  private assertion(): string {
    const now = Math.floor(Date.now() / 1000);
    const encode = (value: unknown) => Buffer.from(JSON.stringify(value)).toString('base64url');
    const header = encode({ alg: 'RS256', typ: 'JWT' });
    const claims = encode({ iss: this.config.clientId, sub: this.config.clientId, aud: this.config.issuer, iat: now, exp: now + 60, jti: randomUUID() });
    const signer = createSign('RSA-SHA256');
    signer.update(`${header}.${claims}`);
    signer.end();
    return `${header}.${claims}.${signer.sign(this.privateKey).toString('base64url')}`;
  }

  private async mint(): Promise<CachedToken> {
    const response = await fetch(this.config.tokenUrl, {
      method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams({
        grant_type: 'client_credentials',
        client_id: this.config.clientId,
        client_assertion_type: 'urn:ietf:params:oauth:client-assertion-type:jwt-bearer',
        client_assertion: this.assertion(),
      }),
      redirect: 'error',
      signal: AbortSignal.timeout(10000),
    });
    if (!response.ok) throw new Error('OIDC service authentication failed');
    if (response.headers.get('content-length') && Number(response.headers.get('content-length')) > 64 * 1024) throw new Error('OIDC service authentication response exceeds 64 KiB');
    if (!response.body) throw new Error('OIDC service authentication returned no response body');
    const reader = response.body.getReader();
    const chunks: Uint8Array[] = [];
    let size = 0;
    while (true) {
      const next = await reader.read();
      if (next.done) break;
      size += next.value.byteLength;
      if (size > 64 * 1024) { await reader.cancel(); throw new Error('OIDC service authentication response exceeds 64 KiB'); }
      chunks.push(next.value);
    }
    const { value, expiresIn } = validateServiceTokenResponse(JSON.parse(Buffer.concat(chunks).toString('utf8')));
    return { value, usableUntil: Date.now() + Math.max(0, expiresIn - 30) * 1000 };
  }

  async token(): Promise<string> {
    if (this.cached && this.cached.usableUntil > Date.now()) return this.cached.value;
    this.pending ??= this.mint().then((token) => { this.cached = token; return token.value; }).finally(() => { this.pending = undefined; });
    return this.pending;
  }
}
