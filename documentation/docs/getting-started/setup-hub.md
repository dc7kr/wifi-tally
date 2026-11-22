# Set up the hub

## Requirements

You need a computer that runs the Hub.

The application is rather lightweight, so there are no special requirements here
and it could run other applications in parallel. The Hub has a web interface for
monitoring and configuration that can be shown on any browser that can connect to
the computer.

To keep latency to a minimum it would be perfect if this computer uses a wired
connection.

## Setup

The hub is a single program that does not need anything else to be installed.
[Download it](../download.md) for your operating system, extract the archive and start it:

````bash
./vtally-hub
````

On Windows double-click `vtally-hub.exe`.

Point your browser to the IP of your computer on port {{ hub_default_port }}, for instance http://127.0.0.1:{{ hub_default_port }} if
you are on the same machine. You should see a screen similar to this

![alt text](../images/tally-hub.png "Tally Hub")

Select `Configuration` in the navigation and configure the settings for your Video Mixer.

!!! info
    The Tallies, their assigned channels and your configuration are automatically saved in `.wifi-tally.json`
    in your home directory. So the configuration is restored when the Hub is restarted. 

## Conclusion

When done, go to the next step to [set up a Web Tally](setup-web-tally.md).
