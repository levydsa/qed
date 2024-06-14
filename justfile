
set dotenv-load

sources := ```
	fd . templates/ -t f
	echo "global.css"
```

target_image := "registry.fly.io/kill-ee-dee:latest"
format := "{{ sql . \"  \" }}"
turso_url := "libsql+wss://${TURSO_SUBDOMAIN}.turso.io?authToken=${TURSO_TOKEN}"

apply-schema:
	atlas schema apply \
		--dev-url "sqlite://dev?mode=memory" \
		--url "{{turso_url}}" \
		--to file://schema.sql

inspect:
	atlas schema inspect \
		--format '{{'{{ sql . "  " }}'}}' \
		--url "{{turso_url}}"

image:
	#!/usr/bin/env bash
	nix build .#container
	image=$(docker image load -i result --quiet | sed -n '$p' | cut -d ':' -f 2- | tr -d '[:space:]')
	docker tag "$image" {{ target_image }}

deploy: image
	docker push {{ target_image }}
	fly deploy

js out="./assets/js/":
	mkdir -p {{ out }} 
	cp -r ./node_modules/htmx.org/dist/.        {{ out }}
	cp -r ./node_modules/hyperscript.org/dist/. {{ out }}
	cp -r ./node_modules/katex/dist/.           {{ out }}

build: tailwind js
	cargo build

tailwind out="./assets/tw.css":
	tailwindcss -i global.css -o {{ out }}

watch: js
	fd . -e rs | entr -sr 'cargo run' & 
	tailwindcss --watch -i global.css -o assets/tw.css

run: tailwind
	cargo run
