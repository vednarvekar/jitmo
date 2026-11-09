"""Entity pools for the synthetic corpus (written from scratch; nothing copied from the SP2 blind set).

Every pool is a plain list of strings. `gen_data.py` draws from them.
"""

APPS = """Spotify WhatsApp Slack Zoom Teams Chrome Firefox Edge Safari Notion Discord Telegram Signal Skype Outlook Excel Word
PowerPoint OneNote Evernote Trello Asana Figma Photoshop Illustrator Premiere Blender VLC Netflix Prime Photos Calendar Calculator
Notepad Terminal VSCode Visual Studio Code Docker Steam Epic Games Twitter Instagram Facebook Snapchat Reddit Pinterest Maps Gmail
Drive Dropbox OneDrive Paint Camera Clock Settings Files Explorer Task Manager Sticky Notes Teams Webex Meet Jira Confluence Anki
Duolingo Kindle Audible Shazam Tinder Uber Ola Swiggy Zomato Paytm PhonePe GPay Obs Studio Audacity GIMP Sublime Postgres Pgadmin
Insomnia Linear Miro Loom Canva Bear Todoist Things Pocket Brave Opera Tor Thunderbird Winamp iTunes Music Podcasts Weather Mail""".split()
APPS = [
    a for a in APPS
    if a not in {"Obsidian", "Postman", "Code", "Studio", "Task", "Manager", "Sticky", "Notes", "Epic", "Games", "Visual", "Obs"}
] + ["VS Code", "Visual Studio", "Task Manager", "Sticky Notes", "OBS Studio", "File Explorer", "Google Chrome", "Epic Games", "Apple Music", "YouTube Music"]
APPS += ["Zoom", "Teams", "Slack", "Chrome"]

# sites that are websites (not apps) in the labelling convention
SITES = [
    "Amazon", "Flipkart", "Myntra", "eBay", "Etsy", "Wikipedia", "GitHub", "Stack Overflow", "Reddit", "LinkedIn", "IMDb", "Quora",
    "Medium", "Zillow", "Booking.com", "Expedia", "Skyscanner", "MakeMyTrip", "IRCTC", "Nykaa", "Ajio", "Meesho", "BigBasket",
    "Walmart", "Target", "Best Buy", "Craigslist", "Pinterest", "Yelp", "TripAdvisor", "Coursera", "Udemy", "Khan Academy", "BBC",
    "CNN", "NDTV", "The Verge", "TechCrunch", "Product Hunt", "Dribbble", "Behance", "OLX", "Indeed", "Naukri", "Glassdoor",
    "Substack", "Twitch", "Hulu", "Hotstar", "Genius", "Letterboxd", "Rotten Tomatoes", "Cricbuzz", "ESPN", "Moneycontrol",
    "Wolfram Alpha", "Google Maps", "Google Docs", "Google Drive", "Netflix", "YouTube", "Twitter", "Gmail", "Zomato", "Swiggy",
    "Lichess", "Duolingo", "Notion", "Figma", "Canva", "Trello", "Airtable", "Hacker News", "Lobsters", "the BBC", "arxiv.org",
    "bing.com", "duckduckgo.com", "nytimes.com", "reuters.com", "amazon.in", "flipkart.com", "github.com",
]

FIRST = """Aarav Aditi Aditya Ajay Akash Akshay Alok Amit Amrita Ananya Anil Anita Anjali Ankit Anna Arjun Arun Asha Ashok Bharat Chitra
Deepa Deepak Dev Dhruv Divya Farhan Gaurav Geeta Gopal Hari Harsh Isha Ishaan Jaya Jyoti Kabir Kavita Kiran Krishna Kunal Lakshmi Lata
Madhav Maya Meena Meera Mohan Mohit Nandini Naveen Neelam Neha Nikhil Nisha Pallavi Pankaj Pooja Prakash Pranav Preeti Priya Rahul
Raj Rajesh Rakesh Ramesh Ravi Rhea Rohan Rohit Ruchi Sachin Sagar Sameer Sanjay Sanya Sapna Sarita Satish Seema Shreya Shruti Siddharth
Simran Sneha Sonia Suman Sunil Tara Tarun Uma Usha Varun Vikas Vikram Vinay Vivek Yash Zoya Alice Ben Carlos Chloe Daniel David
Emily Emma Ethan Grace Hannah Jack James Jessica John Kevin Laura Liam Lucas Maria Mark Mike Nina Olivia Paul Rachel Sam Sarah Sophie
Tom Victoria William Zack Maya Omar Fatima Yusuf Aisha Hassan Ali Mei Chen Yuki Hiro Ingrid Lars Pablo Sofia Ivan Natasha""".split()
FIRST = [n for n in FIRST if n not in {"Rishi", "Tanya", "Mahesh", "Leela", "Farah", "Imran", "Vandana", "Sourav", "Priyanka", "Joseph", "Suresh", "Zara", "Karthik"}]
SURNAMES = ["Sharma", "Patel", "Singh", "Gupta", "Mehta", "Reddy", "Nair", "Iyer", "Khan", "Das", "Rao", "Kapoor", "Joshi", "Smith", "Brown", "Lee", "Garcia", "Miller", "Wilson", "Clark"]
RELATIONS = ["mom", "dad", "mum", "mother", "father", "bro", "sis", "bhai", "didi", "boss", "wife", "husband", "my brother", "my sister", "my wife",
             "my husband", "my boss", "my manager", "grandma", "grandpa", "uncle", "aunt", "mama", "papa", "the landlord", "my roommate",
             "my cousin", "my mom", "my dad", "my friend", "the plumber", "my dentist", "my teacher", "bestie", "partner", "the boss", "coach"]
GROUPS = ["the family group", "the ops group", "the design team", "the project chat", "the book club", "the football group", "the office group",
          "the college gang", "the neighbours group", "our flatmates chat", "the marketing team", "the dev channel", "the standup channel",
          "the parents group", "the trip planning group", "the wedding group", "the sales team", "the alumni group", "my team", "the hr group"]
SENDERS = ["accounts", "support", "HR", "the bank", "Amazon", "Google", "LinkedIn", "Swiggy", "Zomato", "my landlord", "the school",
           "the airline", "the insurance company", "payroll", "the newsletter", "GitHub", "Uber", "my professor", "the recruiter"]

ARTISTS = """Arijit Singh|Taylor Swift|Coldplay|Adele|Drake|Ed Sheeran|Billie Eilish|The Weeknd|AR Rahman|Shreya Ghoshal|Badshah|Diljit Dosanjh|Sidhu Moose Wala|
Kishore Kumar|Mohammed Rafi|Queen|The Beatles|Pink Floyd|Led Zeppelin|Imagine Dragons|Linkin Park|Eminem|Kanye West|Rihanna|Bruno Mars|Ariana Grande|
Post Malone|Harry Styles|Lizzo|Sia|Maroon 5|Shakira|Enrique Iglesias|Neha Kakkar|Sonu Nigam|Atif Aslam|Jubin Nautiyal|Darshan Raval|KK|Anuv Jain|Prateek Kuhad|
Ritviz|Divine|Raftaar|Honey Singh|Guru Randhawa|Sunidhi Chauhan|Alka Yagnik|Udit Narayan|Kumar Sanu|Lata Didi|Asha Bhosle|Hariharan|Pritam Da|Vishal Mishra|
Armaan Malik|Tulsi Kumar|Dhvani Bhanushali|Lil Nas X|Doja Cat|SZA|Olivia Rodrigo|Dua|Metallica|Nirvana|AC/DC|Guns N' Roses|Green Day|Oasis|Radiohead|
Daft Punk|Calvin Harris|Martin Garrix|Avicii|Marshmello|Zedd|Kygo|Alan Walker|Hans Zimmer|Ludovico Einaudi|Yiruma|Frank Sinatra|Elvis|Michael Jackson|Madonna|
Prince|ABBA|Bob Marley|Bob Dylan|Johnny Cash|Fleetwood Mac|Mumford and Sons|Arctic Monkeys|Lana Del Rey|Halsey|Charlie Puth|Shawn Mendes|Justin Bieber|
Lewis Capaldi|Dean Lewis|Tame Impala|Gorillaz|Jonas Brothers|BTS|Blackpink|Twice|Karan Aujla|AP Dhillon|Jasleen Royal|Monali Thakur|Benny Dayal|Shaan|Mika Singh|
Papon|Nucleya|Zakir Khan|Vir Das|Dhruv Rathee|Mrbeast|Linus Tech Tips|Veritasium|Kurzgesagt|Marques|Casey Neistat|Mark Rober|Ali Abdaal|Thomas Frank|Tom Scott|
Vsauce|CGP Grey|Computerphile|Sentdex|Fireball|Technical Guruji|CarryMinati|Triggered Insaan|Ranveer Allahbadia|Beerbiceps|Sandeep Maheshwari|Harkirat|Striver|
Traversy Media|The Primeagen|Theo|Wendover|Mr Whose The Boss|Dude Perfect|Peter McKinnon""".replace("\n", "").split("|")
SONGS = """Tum Hi Ho|Blinding Lights|Bohemian Rhapsody|Hotel California|Believer|Perfect|Someone Like You|Viva La Vida|Stairway to Heaven|Smells Like Teen Spirit|
Kesariya|Channa Mereya|Raataan Lambiyan|Apna Bana Le|Kal Ho Naa Ho|Chaiyya Chaiyya|Jai Ho|Senorita|Despacito|Levitating|Bad Guy|Old Town Road|Lose Yourself|
Sunflower|Stay|Heat Waves|As It Was|Anti Hero|Flowers|Industry Baby|Peaches|drivers license|Kiss Me More|Good 4 U|Starboy|Save Your Tears|Yellow|The Scientist|
Fix You|Clocks|Counting Stars|Radioactive|Demons|Thunder|In The End|Numb|Faded|Closer|Roses|Shape of My Heart|Thinking Out Loud|Photograph|Castle on the Hill|
Imagine|Let It Be|Yesterday|Hey Jude|Wonderwall|Creep|Mr Brightside|Sweet Child O Mine|Back in Black|Thunderstruck|Livin on a Prayer|Dont Stop Believin|
Dil Chahta Hai|Tujhe Kitna Chahne Lage|Ve Kamleya|Kabira|Ilahi|Agar Tum Saath Ho|Phir Le Aya Dil|Ae Dil Hai Mushkil|Tera Ban Jaunga|Pehla Nasha|Lag Ja Gale|
Kun Faya Kun|Maahi Ve|Dil Diyan Gallan|Naatu Naatu|Srivalli|Butta Bomma|Manike Mage Hithe|Chaand Baaliyan|Tere Vaaste|Husn|Heeriye|Pasoori|Excuses|Brown Munde|
Shape of Water|Golden Hour|Cruel Summer|Shake It Off|Love Story|Blank Space|Bad Habits|Happier Than Ever|Ocean Eyes|Watermelon Sugar|Adore You""".replace("\n", "").split("|")
GENRES = ["jazz", "lofi", "lo-fi beats", "classical", "rock", "pop", "hip hop", "bollywood hits", "90s hits", "workout music", "ghazals", "sufi", "country",
          "chill music", "focus music", "party songs", "old hindi songs", "piano music", "study beats", "acoustic covers", "retro songs", "edm", "rap", "blues", "metal"]
PLAYLISTS = ["my workout playlist", "the road trip playlist", "my liked songs", "discover weekly", "my morning playlist", "the chill mix", "my gym playlist",
             "release radar", "daily mix", "my focus playlist"]
VIDEO_THINGS = ["latest video", "newest video", "new vid", "latest upload", "recent video", "newest episode", "latest episode", "new upload"]

FILE_NOUNS = """resume|CV|cover letter|budget|invoice|invoices|bank statement|tax return|salary slip|payslip|lease|rent agreement|passport scan|visa letter|boarding pass|
flight ticket|insurance policy|medical report|lab results|prescription|contract|offer letter|presentation|slides|pitch deck|meeting notes|minutes|project plan|roadmap|
report|proposal|spec|thesis|essay|assignment|syllabus|timetable|schedule|photo|photos|screenshot|screenshots|selfie|wallpaper|logo|mockup|design mockup|wireframe|
recording|voice memo|podcast|video|movie|song|ebook|manual|brochure|flyer|receipt|receipts|bill|electricity bill|water bill|phone bill|quote|estimate|checklist|todo list|
shopping list|recipe|itinerary|agenda|newsletter|draft|final draft|backup|spreadsheet|expense report|timesheet|sales figures|quarterly numbers|cheat sheet|notes|diary|
certificate|degree certificate|marksheet|id card|aadhaar copy|pan card|driving licence|portfolio|gym schedule|class notes|lecture slides|research paper|term paper""".replace("\n", "").split("|")
FILE_ADJ = ["latest", "last", "new", "old", "final", "updated", "signed", "scanned", "draft", "recent", "first", "original"]
FILE_TOPIC = ["Q1", "Q2", "Q3", "Q4", "2023", "2024", "2025", "march", "april", "june", "december", "project", "client", "marketing", "sales", "board", "team", "annual", "monthly", "weekly", "wedding", "holiday", "birthday", "kitchen", "home loan", "car", "tax"]
FILE_EXT = ["pdf", "docx", "xlsx", "pptx", "txt", "csv", "png", "jpg", "mp3", "mp4", "zip", "md", "json", "py", "key", "numbers", "pages", "odt"]
FILE_STEM = ["budget", "report", "notes", "invoice", "resume", "summary", "draft", "presentation", "contract", "photo", "proposal", "meeting minutes",
             "sales", "todo", "plan", "schedule", "backup", "design", "final version", "q2 numbers", "cv", "thesis", "chapter 3", "travel plan",
             "tax 2024", "readme", "script", "data export", "ideas", "brief", "agenda", "expenses", "timeline", "specs", "pricing"]
FILES_PLURAL = ["these pdfs", "these files", "these screenshots", "all my invoices", "my photos", "the old backups", "those documents", "the receipts", "my downloads",
                "all the pdfs", "the images", "these photos", "all my receipts", "those spreadsheets", "the scanned docs", "everything from yesterday"]
FOLDERS = ["Downloads", "Documents", "Desktop", "Pictures", "Music", "Videos", "Projects", "Work", "Personal", "Taxes", "Invoices", "Receipts", "Finance", "Archive",
           "Backup", "Memes", "Travel", "School", "College", "Resumes", "Contracts", "Reports", "Screenshots", "Misc", "Old Stuff", "Photos 2024", "Family", "Health",
           "Insurance", "Recipes", "Books", "Design", "Clients", "Admin", "To Sort", "Scans", "Receipts 2024", "Project X", "Side Projects", "Dev", "Wedding", "Home"]

EVENTS = """standup|stand-up|daily standup|team sync|1:1 with Harsh|one on one with my manager|design review|sprint planning|sprint review|retro|retrospective|all hands|
town hall|board meeting|client call|investor call|demo|product demo|interview|job interview|phone screen|dentist appointment|doctor appointment|checkup|physio|yoga|
yoga class|gym|gym session|workout|run|football|cricket practice|swim|piano lesson|guitar class|tuition|exam|midterm|viva|presentation|lunch with Sam|dinner with the Mehtas|
coffee with Anita|coffee chat|birthday party|anniversary dinner|wedding|housewarming|movie night|game night|date night|haircut|salon|car service|vet appointment|
parent teacher meeting|webinar|workshop|conference call|office hours|lab session|deep work|focus time|writing|reading time|study session|therapy|meditation|call with the bank|
flight to Delhi|train to Pune|pickup|school pickup|grocery run|payday|rent due|bill payment|team lunch|offsite|happy hour|book club|quiz night|karaoke|trek|hackathon|code review|planning meeting|kickoff""".replace("\n", "").split("|")

PLACES = """Mumbai|Delhi|Bangalore|Bengaluru|Chennai|Kolkata|Hyderabad|Pune|Ahmedabad|Jaipur|Lucknow|Chandigarh|Goa|Kochi|Mysore|Indore|Bhopal|Nagpur|Surat|Patna|Shimla|Manali|Ooty|
London|Paris|Berlin|Madrid|Rome|Amsterdam|Dublin|Lisbon|Vienna|Prague|Zurich|Oslo|Stockholm|Tokyo|Osaka|Seoul|Beijing|Shanghai|Singapore|Bangkok|Dubai|Doha|Istanbul|Cairo|Nairobi|Lagos|
New York|San Francisco|Los Angeles|Chicago|Boston|Seattle|Austin|Miami|Denver|Toronto|Vancouver|Montreal|Mexico City|Sao Paulo|Buenos Aires|Sydney|Melbourne|Auckland|Cape Town|
Kathmandu|Colombo|Dhaka|Karachi|Lahore|Hanoi|Kuala Lumpur|Jakarta|Manila|Hong Kong|Taipei|Edinburgh|Manchester|Brighton|Portland|Phoenix|Houston|Dallas|Atlanta""".replace("\n", "").split("|")
PLACES = [p for p in PLACES if p not in {"Kolkata", "Hyderabad", "San Francisco", "Mysore"}] + ["Gurgaon", "Noida", "Thane", "Vizag", "Salt Lake City", "St Louis"]
COUNTRIES = ["Australia", "Canada", "France", "Japan", "Brazil", "Germany", "Egypt", "Peru", "Kenya", "Norway", "Italy", "Spain", "Chile", "Vietnam", "Mexico", "Argentina", "Thailand", "Sweden"]

SETTINGS = ["volume", "brightness", "wifi", "wi-fi", "bluetooth", "airplane mode", "flight mode", "do not disturb", "dnd", "focus mode", "night mode", "dark mode",
            "night light", "battery saver", "power saver", "hotspot", "location", "notifications", "silent mode", "quiet mode", "sound", "mic", "microphone", "camera",
            "screen brightness", "ringer", "bass", "nfc", "vpn", "auto rotate"]

SEARCH_NOUN = """best budget phones|cheap flights to goa|how to tie a tie|python list comprehension|weather in paris|pasta recipes|vegan lasagna recipe|easy dinner ideas|
laptop deals|noise cancelling headphones|running shoes for flat feet|how to fix a leaking tap|what is inflation|symptoms of flu|how to meditate|how many calories in an apple|
best sci fi movies|top 10 podcasts|nearest atm|pharmacy near me|coffee shops open now|how to remove a stain from a shirt|history of the roman empire|cricket scores|
nba standings|bitcoin price|usd to inr rate|train timings mumbai to pune|visa requirements for japan|diy bookshelf|learn guitar chords|beginner yoga routine|
ielts tips|resume templates|how to write a cover letter|rust ownership explained|react hooks tutorial|docker compose example|linux commands cheat sheet|why is the sky blue|
how do vaccines work|garden pests|houseplants for low light|mattress reviews|standing desk under 20000|iphone 15 review|pixel vs iphone|solar panel cost|electric cars in india|
tax saving options|mutual funds for beginners|how to apply for a passport|driving test tips|baby names|wedding gift ideas|birthday party games|how to bake bread|sourdough starter|
street food in bangkok|hiking trails near me|camping checklist|local news|stock market today|movie showtimes|lofi study mix|a wireless mouse|gaming chairs|usb c hub|monitor arms""".replace("\n", "").split("|")
SEARCH_TOPIC = ["the rental agreement", "the contract renewal", "the visa appointment", "the budget", "the invoice", "the quarterly report", "the offer letter", "the project deadline",
                "the flight booking", "the team offsite", "the salary revision", "the insurance claim", "the delivery", "the refund", "the interview schedule", "the lease", "the hackathon",
                "the new policy", "our trip to goa", "the wedding plan", "the school fees", "my order", "the server outage", "the launch plan", "the tax filing"]

MESSAGES = """I am running late|running 10 mins late|on my way|reached home|landed safely|call me when you can|call me back|can we talk later|are you free tonight|
dinner is at 8|happy birthday|congrats on the new job|thanks a lot|thank you so much|sorry I missed your call|I will be there in five|see you tomorrow|let's catch up this weekend|
the meeting got moved to 4|the report is ready|please send me the files|I'll send it by evening|did you eat|good morning|good night|sleep well|miss you|love you|
where are you|are you coming|I'm outside|pick me up at 6|don't forget the keys|bring milk on your way|we are out of eggs|the demo is pushed to monday|deploy is done|build is green|
the client approved the design|can you review my pull request|lunch at 1?|let's do pizza tonight|I'll call you after the meeting|got the tickets|got it, thanks|sounds good|ok see you then|
yes I can make it|no I can't make it tomorrow|i need a few more days|please share the notes|waiting for your reply|ping me when you're free|the wifi password is on the fridge|
we won!|take care|get well soon|best of luck for the exam|meeting starts in 10|join the call|I'll be late for standup|the package arrived|power cut here|traffic is terrible|
I just left|leaving now|heading out now|just woke up|can't talk right now|in a meeting, will call back|please call the plumber|water the plants|the doctor said it's fine|rain check?|
what time should I come|do you need anything from the store|mom wants you to call her|we should plan a trip|check your email|I forwarded the invoice|payment done|cab is here|
I'll bring dessert|the movie starts at 9|save me a seat|are we still on for tonight|hope you feel better|just checking in|thinking of you|good luck today|nice work on the launch|
can you cover for me tomorrow|I owe you one|my flight is delayed|boarding now|I've reached the hotel|the key is under the mat|call the bank|remind me about the dentist""".replace("\n", "").split("|")
MSG_LEAD = ["", "", "", "hey ", "hi ", "ok ", "so "]
MSG_SOFT = ["pls call me", "need your help", "we need to talk", "are you around", "can you pick up the kids", "call me asap", "be there soon", "share your location"]

REMIND_TASKS = """call mom|send the report|take vitamins|pay the electricity bill|renew the passport|water the plants|buy milk|submit the assignment|book the tickets|call the dentist|
take out the trash|drink water|stretch|check the oven|pick up the laundry|feed the cat|refill the prescription|email the client|back up the laptop|charge the earbuds|
call the plumber|file the taxes|order groceries|check on grandma|take the medicine|join the webinar|post the invoice|wish Anil happy birthday|buy a gift for Meera|reply to the recruiter|
update the timesheet|clean the kitchen|walk the dog|renew the car insurance|pay rent|collect the parcel|get a haircut|print the boarding pass|lock the back door|review the pull request|
send the newsletter|prepare for the interview|read chapter 4|practice guitar|call the bank|top up the metro card|cancel the free trial|book a table|do the dishes|meditate""".replace("\n", "").split("|")
TASK_VERBS = ["call", "email", "text", "pay", "buy", "send", "book", "order", "pick up", "renew", "check", "submit", "water", "feed", "clean", "update", "file", "print", "cancel", "return"]

DAYS = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"]
MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"]

CONVERT_UNITS = [("km", "miles"), ("miles", "km"), ("kg", "pounds"), ("pounds", "kg"), ("inches", "centimetres"), ("feet", "metres"), ("celsius", "fahrenheit"),
                 ("litres", "gallons"), ("cups", "ml"), ("ounces", "grams"), ("usd", "inr"), ("hours", "minutes"), ("metres", "feet"), ("cm", "inches")]
WORD_NUMS = ["one", "two", "three", "four", "five", "six", "seven", "eight", "ten", "twelve", "fifteen", "twenty", "thirty", "forty five"]
