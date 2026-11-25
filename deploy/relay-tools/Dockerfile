FROM node:24.22.3-alpine@sha256:116b800c3754b440ee0aa3ed2f20f5bf3d99a554bfd9adeea09ea8017f6e85f5
WORKDIR /app
COPY package.json lib.mjs meter.mjs proxy.mjs access.mjs ./
RUN mkdir -p /data /flags && chown node:node /data /flags
USER node
CMD ["node", "meter.mjs"]
