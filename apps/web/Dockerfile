FROM oven/bun:1.3.14-alpine AS builder

WORKDIR /workspace
COPY package.json bun.lock ./
COPY apps/web/package.json ./apps/web/package.json
RUN bun install --frozen-lockfile
COPY apps/web ./apps/web

ARG BACKEND_URL=http://api:8080
ENV BACKEND_URL=${BACKEND_URL}
RUN bun run build:web

FROM oven/bun:1.3.14-alpine AS runtime

WORKDIR /workspace
COPY --from=builder /workspace/package.json /workspace/bun.lock ./
COPY --from=builder /workspace/node_modules ./node_modules
COPY --from=builder /workspace/apps/web ./apps/web

ENV NODE_ENV=production
ENV PORT=3000
EXPOSE 3000

CMD ["bun", "run", "--cwd", "apps/web", "start"]

