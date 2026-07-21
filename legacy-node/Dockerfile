FROM node:22-alpine AS builder
WORKDIR /app
COPY src/discord-bot/package.json src/discord-bot/package-lock.json ./
RUN npm ci
COPY src/discord-bot/ .
COPY --chown=node:node src/frontend/public/data/paladins-talent-reference.json \
  src/frontend/public/data/paladins-card-reference.json \
  src/frontend/public/data/paladins-loadout-frame-reference.json \
  src/frontend/public/data/champion-data.json \
  ./data/
COPY --chown=node:node src/frontend/public/images/champions/Talent*.png ./assets/champions/
COPY --chown=node:node src/frontend/public/images/champions/Banner_Androxus.png ./assets/champions/Banner_Androxus.png
COPY --chown=node:node src/frontend/public/images/cards/Card_Sprint.png ./assets/cards/Card_Sprint.png
COPY --chown=node:node src/frontend/public/images/cards/frames/*.png ./assets/cards/frames/
RUN npm run build

FROM node:22-alpine AS runtime
WORKDIR /app
ENV NODE_ENV=production
RUN apk add --no-cache chromium fontconfig font-inter ttf-dejavu
ENV PALADINSCAT_CHROMIUM_PATH=/usr/bin/chromium-browser \
    HOME=/tmp \
    XDG_CACHE_HOME=/tmp
RUN chown node:node /app
USER node
COPY --chown=node:node --from=builder /app/package.json /app/package-lock.json ./
RUN npm ci --omit=dev && npm cache clean --force
COPY --chown=node:node --from=builder /app/dist ./dist
COPY --chown=node:node --from=builder /app/data ./data
COPY --chown=node:node dev/prototypes/match-result-scoreboard.html ./templates/match-result-scoreboard.html
COPY --chown=node:node dev/prototypes/cheater-police-line.svg ./templates/cheater-police-line.svg
EXPOSE 3020
CMD ["node", "dist/src/index.js"]
