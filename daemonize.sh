#!/bin/bash

# Check if the script is run with sudo
if [ "$EUID" -ne 0 ]; then
  echo "Please run this script with sudo."
  exit 1
fi

APP_NAME="jikan"

# Check if the app exists in Cargo's bin directory
if [ ! -f "$HOME/.cargo/bin/$APP_NAME" ]; then
  echo "Error: $APP_NAME not found in Cargo's bin directory."
  echo "Please make sure you've installed it with 'cargo install $APP_NAME'."
  exit 1
fi

# Create the plist file
cat << EOF > /Library/LaunchDaemons/com.user.$APP_NAME.plist
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.user.$APP_NAME</string>
    <key>ProgramArguments</key>
    <array>
        <string>$HOME/.cargo/bin/$APP_NAME</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardErrorPath</key>
    <string>/tmp/$APP_NAME.err</string>
    <key>StandardOutPath</key>
    <string>/tmp/$APP_NAME.out</string>
</dict>
</plist>
EOF

# Set correct ownership and permissions
chown root:wheel /Library/LaunchDaemons/com.user.$APP_NAME.plist
chmod 644 /Library/LaunchDaemons/com.user.$APP_NAME.plist

# Load the daemon
launchctl load /Library/LaunchDaemons/com.user.$APP_NAME.plist

# Start the daemon
launchctl start com.user.$APP_NAME

echo "Daemon for $APP_NAME has been set up and started."
echo "You can find logs in /tmp/$APP_NAME.out and /tmp/$APP_NAME.err"