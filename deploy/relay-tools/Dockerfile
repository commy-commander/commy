FROM node:24-alpine
WORKDIR /app
COPY package.json lib.mjs meter.mjs proxy.mjs ./
RUN mkdir -p /data /flags && chown node:node /data /flags
USER node
CMD ["node", "meter.mjs"]
