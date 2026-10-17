# Notifications

Niles tells you things two ways: out loud through a satellite, and as a
notification on your phone or computer.

## On your phone or computer

When Niles writes something for you in the background, a travel plan or a
packing list, it can tell you once it is ready. Tap the notification and
the app opens on it. Niles sends one when:

- something it was writing for you is ready: "Your {title} is ready."
- it did not finish: "Your {title} didn't finish. Open it to try again."
- a page you asked for is ready: "The page for your {title} is ready."
- someone shares something with you: "{name} shared "{title}" with you."
- your training plan changed after a run: "Your {title} changed after your
  run.", with the coach's note

Each item under [Saved](saved.md) has a **Tell me when it's ready** switch,
on by default, that covers all of these except a share. There is no
notification for an approval waiting or for a timer.

There is nothing to set up on the server beyond what Niles already needs:
its database and its secrets key. Notifications go out from Niles to
Apple's, Google's or Mozilla's push service, so Niles does not have to be
reachable from outside the house for them to arrive.

### Turning them on

Each device is turned on by itself, from **My profile → Notifications →
On this device**, or by tapping **Notify me when it's done** under
something that is being written. Your browser asks once whether Niles may
send notifications; say yes.

**Send a test** on the same card sends one to every device you have turned
on, which is the quickest way to check a new one works. The card also
lists your devices, with when each was added and last sent to.

### On an iPhone or iPad

iOS only lets an app on the Home Screen show notifications (iOS 16.4 or
later), not a page open in Safari. So first:

1. Open Niles in Safari.
2. Tap **Share**, then **Add to Home Screen**.
3. Open Niles from the Home Screen, sign in, and turn notifications on
   from **My profile**.

### If you said no

The browser remembers, and only you can change it there:

- **iPhone or iPad:** Settings → Notifications → Niles.
- **Android:** long-press the Niles icon → App info → Notifications.
- **A computer:** the site settings by the address bar, then
  Notifications → Allow.

Then turn **On this device** on again.

### Turning them off

Switch **On this device** off, or remove a device from the list on the
same card. A device where you turned notifications off in the browser, or
removed the app, is forgotten by itself the next time Niles tries it.

## Out loud

Some things Niles says without being asked:

- what an automation's **Say something** step says, in the room it names
- announcements from add-ons that are allowed to speak
- something you asked Niles to announce in a room

An announcement is spoken in the room it names. One without a room is an
announcement for the house, spoken in every room that has a satellite
under **Settings → Satellites**. These have no room:

- the nemlig.com reminder that a grocery delivery is on its way
- an automation's **Say something** left on **Wherever Niles would**
- a timer set from a satellite that is not listed under **Settings →
  Satellites**
- Linear telling Niles that an issue moved (see [Add-ons](add-ons.md))

With no satellites listed, an announcement without a room is recorded but
not spoken.

### Priorities and quiet hours

Each announcement is **Routine**, **Important** or **Urgent**. Routine is
the default.

Quiet hours are a window, such as 22:00 to 07:00, set in Niles's config
file by whoever runs it:

```toml
[notifications.quiet_hours]
start = "22:00"
end = "07:00"
```

There are no quiet hours unless they are set. During them:

| Priority | During quiet hours |
|---|---|
| Routine | Not spoken. It is not saved for later either. |
| Important | Spoken as usual |
| Urgent | Spoken as usual |

An add-on can only speak at Urgent if its approval screen said "even
during quiet hours".

### On the TV

With the LG TV add-on, **Announcements on the TV** shows what Niles says
aloud on the screen as well. It also shows announcements that have no
room.

### What did I miss

Ask "what just happened" and Niles reads its recent announcements, the
last 50. With a database they are kept there, so a restart does not lose
them. Without one, Niles forgets them when it restarts.
