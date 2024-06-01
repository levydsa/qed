
sources := ```
	fd . templates/ -t f
	echo "global.css"
```

js:
	mkdir -p ./assets/js/
	cp -r ./node_modules/htmx.org/dist/* ./assets/js/
	cp -r ./node_modules/hyperscript.org/dist/* ./assets/js/
	cp -r ./node_modules/katex/dist/* ./assets/js/

build: tailwind js 
	cargo build

tailwind:
	bun x tailwindcss -i global.css -o assets/tw.css

watch: js
	fd . -e rs | entr -sr 'cargo run' & 
	bun x tailwindcss --watch -i global.css -o assets/tw.css

run: tailwind
	cargo run
