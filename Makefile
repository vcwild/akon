.PHONY: install-deps

# Install system dependencies required for the project
all:
	sudo dnf install -y openconnect-devel dbus-devel pkgconf-pkg-config
