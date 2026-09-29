FROM node:22-alpine AS build
WORKDIR /app
COPY package*.json ./
RUN npm ci
COPY . .
RUN npm run build

FROM alpine
COPY --from=build /app/dist /everything/
COPY packages/ /everything/packages/
WORKDIR /everything
RUN mv packages/* . && rm -rf packages
