PREFIX ?= /usr/local
DESTDIR ?=
CARGO ?= cargo

.PHONY: build install uninstall deb test clean

build:
	$(CARGO) build --release

# The askpass helper, the catalog and the logos keep fixed paths (/usr/lib/agent-center, /usr/share/agent-center):
# `agent-center run-system` only trusts root-owned catalog directories at those locations.
install:
	install -Dm755 target/release/agent-center $(DESTDIR)$(PREFIX)/bin/agent-center
	install -Dm755 data/askpass $(DESTDIR)/usr/lib/agent-center/askpass
	sed -i 's|/usr/bin/agent-center|$(PREFIX)/bin/agent-center|' $(DESTDIR)/usr/lib/agent-center/askpass
	install -d $(DESTDIR)/usr/share/agent-center/catalog
	install -m644 catalog/*.toml $(DESTDIR)/usr/share/agent-center/catalog/
	install -d $(DESTDIR)/usr/share/agent-center/logos
	install -m644 logos/*.svg $(DESTDIR)/usr/share/agent-center/logos/
	install -Dm644 data/agent-center.desktop $(DESTDIR)$(PREFIX)/share/applications/agent-center.desktop
	install -Dm644 docs/logo.svg $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/agent-center.svg

uninstall:
	rm -f $(DESTDIR)$(PREFIX)/bin/agent-center $(DESTDIR)/usr/lib/agent-center/askpass \
		$(DESTDIR)$(PREFIX)/share/applications/agent-center.desktop \
		$(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/agent-center.svg
	rm -rf $(DESTDIR)/usr/share/agent-center
	-rmdir $(DESTDIR)/usr/lib/agent-center

# Builds target/debian/agent-center_<version>_<arch>.deb (needs: cargo install cargo-deb)
deb:
	$(CARGO) deb

test:
	$(CARGO) test --no-default-features

clean:
	$(CARGO) clean
