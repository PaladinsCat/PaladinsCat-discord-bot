FROM node:22-alpine AS builder
WORKDIR /app
COPY src/discord-bot/package.json src/discord-bot/package-lock.json ./
RUN npm ci
COPY src/discord-bot/ .
RUN npm run build

FROM node:22-alpine AS runtime
WORKDIR /app
ENV NODE_ENV=production
RUN apk add --no-cache chromium fontconfig font-inter ttf-dejavu
ENV PALADINSCAT_CHROMIUM_PATH=/usr/bin/chromium-browser \
    HOME=/tmp \
    XDG_CACHE_HOME=/tmp
COPY --from=builder /app/package.json /app/package-lock.json ./
RUN npm ci --omit=dev && npm cache clean --force
COPY --from=builder /app/dist ./dist
COPY dev/prototypes/match-result-scoreboard.html ./templates/match-result-scoreboard.html
RUN chown -R node:node /app
USER node
EXPOSE 3020
CMD ["node", "dist/src/index.js"]
