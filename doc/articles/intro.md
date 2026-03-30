# Intro

My first brush with a PC was two blue Norton Commander panels in 1991. Back then it felt like magic.
Time went on.

Fortran, Clarion, FoxPro, Pascal, Smalltalk, C, Oracle, Java, Postgres, Node.js, TypeScript, Rust…  
MS-DOS, Windows, Linux, macOS…  
Norton Commander, Far Manager, Midnight Commander…

Thirty-plus years of code, wins and flops, companies, coworkers, and screens—and those blue panels never really left the picture.

I’m older now, and pickier. When I miss a feature I actually care about, I can fix it the hard way. I’m not about to rewrite a database or an OS—but for tools and utilities? That’s a different story.

So when the macOS build of Midnight Commander randomly hangs for five seconds, or I have to hit Ctrl+O one time too many, I finally stopped shrugging it off. I wanted something *like* MC—maybe simpler, but fast, and shaped the way I work. For myself. First I did a quick sweep for good alternatives. That’s when it hit me: there isn’t really a console file manager with the same mix of popularity and depth—not on macOS, and not on Linux either. Windows I’ll skip for now; I don’t live there day to day, and Far Manager already exists.

Here are a few questions that stuck with me after that search:

1. What if it weren’t just for Linux and macOS?  
2. What if the design actually reflected how hardware has changed?  
3. What if we kept the classic idea but folded in some fresh UI/UX?  
4. What if the thing were built in a language from the *21st* century, not the 20th?

If you’ve read this far, you’ve probably guessed: I decided to build something new. Those questions *are* the motivation—and the rough roadmap.

The stack is **Rust**—a **systems programming** language at heart (think kernels, runtimes, and tools that sit close to the machine). I won’t give the full sales pitch. It learns from what went wrong before, fixes whole classes of old mistakes more honestly than most alternatives, and still carries forward the good ideas from earlier generations. On top of that it’s built for real systems work: memory-safe without a garbage collector, fast, and practical for desktop and CLI apps—not just slides and benchmarks.

There’s also a great community and a rich crate ecosystem. For the console UI I went all-in on **Ratatui**—and honestly, it’s a joy: a mature, batteries-included TUI toolkit (layouts, widgets, styling, the works) that feels like building a real UI instead of hand-drawing escape codes. It sits on solid terminal backends, the docs and examples actually help, and the project is alive—exactly what you want when you’re not writing a toy demo.

So—meet **Oxide**. The name is a nod to the Rust ecosystem.

I built it with AI in the loop, and honestly it turned out better than I expected—which is why I’m sharing it with you.

Next up: a set of **real stories**—almost little novellas—each one a concrete use case that shows why this thing exists and how it behaves in practice. I hope you’ll read them like vignettes, not a manual.

---

## Switching between the command line and the panels

### The problem

Picture this: you’ve typed a long command, then you realize you need to open a file somewhere else—outside the current panel paths—to double-check a name, a path, or some value before you run it.

### How Midnight Commander handles it

You’d normally use **Enter** to move into folders and open files. But with focus on the command line, **Enter** means “run the command.” So you can’t freely browse the tree and edit the command in one breath: the same key is wired to two different jobs. Typical two-panel managers also steer most keys either toward the line or toward the list, but **Enter** is the painful overlap—you’re stuck choosing between executing and navigating.

### How Oxide handles it

**Command-line focus** and **panel focus** are explicit states. Start typing a command-like character (a letter, digit, etc.) and it goes into the prompt and the app switches to **command-line mode**. Need the panels? Press **Esc**—you’re back on the file list; browse, open the file, verify whatever you needed. Press **Esc** again and you return to the command line with your text and cursor intact. Arrow keys work in command-line mode for editing. No more fighting **Enter** just to peek at a file mid-command.

![Switching between command line and panels in Oxide](images/comm-split.gif)

Maybe you’re wondering why the file names show up so fast in the clip above. That’s on purpose: in **command-line mode**, **F12** inserts the **currently selected file name** from the active panel at the **cursor**—handy when you’re building a command and don’t want to retype paths by hand.

---

## Time to read the command output

### The problem

You run something from the prompt and the **output** is what matters—you want a moment (or more) to read it before the twin panels take over the screen again.

### How Midnight Commander handles it

People usually fall into one of two habits. **Ctrl+O** first to hide the panels and get a clean terminal, then run the command. Or run with the UI as-is and, afterward, hit **Ctrl+O** again to flip between the shell buffer and the file panes. It works, but you’re often **toggling** instead of getting a deliberate pause.

### How Oxide handles it

There’s a setting—**Auto reopen panels after command/executable** (F9 → **General settings**)—that changes what happens *after* the command finishes.

- **On:** Oxide waits for a **configurable delay** (seconds) while you stay on the shell output; a small countdown appears at the **bottom-left** of the terminal. When the timer ends, the panels come back. No rush to read the first screenful.
- **Off:** Closer to classic MC: the output stays up until **you** decide to return; **Ctrl+O** brings the panels back when you’re ready.

![Return to panels after command — delay](images/exec-delay.gif)

---

