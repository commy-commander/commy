FROM node:22-alpine
WORKDIR /app
COPY package.json lib.mjs meter.mjs proxy.mjs ./
# Named volumes copy this ownership on first use
RUN mkdir -p /data /flags && chown node:node /data /flags
USER node
CMD ["node", "meter.mjs"]
