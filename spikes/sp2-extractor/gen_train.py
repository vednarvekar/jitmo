"""Template generator for SP2 fine-tune data. Output lines: ID|text with [label|span]."""
import random, re
random.seed(7)
C=["Rahul","Neha","Kiran","Priya","Amit","Sneha","Rohit","Anjali","Vikram","Pooja","Arjun","Meera","Sanjay","Divya","Karan","Isha","John","Sarah","Mike","Emma","David","Zoya","Aditi","Manish","rahul","neha","priya","amit","mom","dad","wife","bhai","didi","boss","my manager","Dr. Shah","Ravi sir","Sunita ma'am"]
APP=["WhatsApp","Chrome","Spotify","Teams","Outlook","Excel","Word","Notepad","Calculator","Slack","Zoom","Discord","VS Code","Edge","Firefox","Telegram","Paint","Photos","Settings","File Explorer","PowerPoint","Notion","Figma"]
WEB=["youtube","gmail","github","linkedin","amazon","myntra","flipkart","netflix","google maps","reddit","twitter","chatgpt"]
SET=["wifi","bluetooth","dark mode","do not disturb","night light","airplane mode","battery saver","quiet mode","focus mode"]
SETN=["volume","brightness"]
DT=["tomorrow","tonight","today at 5pm","7pm","next monday","on friday","at 9:30 am","tomorrow morning","this evening","in the afternoon","on the 15th","next week","at noon","tomorrow at 8","saturday 6pm","monday morning","at 10"]
DUR=["10 minutes","half an hour","2 hours","45 mins","an hour","5 min","90 seconds","20 minutes","1 hr"]
NUM=["20","50","75","30","100","40","15"]
FILE=["resume","invoice","salary slip","budget.xlsx","report.pdf","lease agreement","presentation","tax return","passport scan","screenshot","notes.txt","cv","project proposal","assignment","photo.jpg","timesheet","offer letter"]
FOLD=["Downloads","Documents","Desktop","Receipts","Work","Pictures","Projects","College","Taxes"]
MEDIA=["Arijit Singh","Taylor Swift","Kesariya","Believer","Ed Sheeran","Honey Singh","lofi beats","AP Dhillon","Shape of You","some jazz","Coldplay","Blinding Lights","Kishore Kumar","Diljit"]
MSG=["I'm running late","call me back","dinner at 8?","meeting moved to 4","happy birthday","send me the file","where are you","I'll be there in 10","pick up milk","see you tomorrow","thanks a lot","can we talk later"]
TASK=["call mom","submit the form","pay the rent","buy groceries","email HR","take medicine","book tickets","water the plants","renew insurance","call the plumber"]
QUERY=["weather in Pune","capital of Canada","who won the world cup","time in London","population of India","meaning of ephemeral","distance to Mumbai"]
PLACE=["Pune","Mumbai","Delhi","London","Bangalore","Goa","Tokyo","New York"]
EVT=["standup","lunch with the team","dentist","interview","1:1 sync","design review","yoga class","client call"]
UNIT=["10 km in miles","5 kg to pounds","100 usd in inr","20 celsius in fahrenheit","3 feet in cm"]
F=lambda l:random.choice(l)
def c(n):return f"[contact|{n}]"
def L(l,x):return f"[{l}|{x}]"
PRE=["","","","please ","pls ","hey ","can you ","could you ","kindly ","quick, ","yo "]
SUF=[""," please",""," now"," asap",""," for me",""]
T=[
lambda:f"tell {c(F(C))} {L('message',F(MSG))}",
lambda:f"message {c(F(C))} {L('message',F(MSG))}",
lambda:f"text {c(F(C))} on {L('app',F(['WhatsApp','Telegram']))} {L('message',F(MSG))}",
lambda:f"send {c(F(C))} a message saying {L('message',F(MSG))}",
lambda:f"mail {c(F(C))} the {L('file',F(FILE))}",
lambda:f"email {L('file',F(FILE))} to {c(F(C))}",
lambda:f"send the {L('file',F(FILE))} to {c(F(C))} on {L('app',F(['WhatsApp','Teams','Outlook','Slack']))}",
lambda:f"call {c(F(C))}",
lambda:f"call {c(F(C))} {L('date_time',F(DT))}",
lambda:f"remind me to {L('message',F(TASK))} {L('date_time',F(DT))}",
lambda:f"remind me {L('date_time',F(DT))} to {L('message',F(TASK))}",
lambda:f"set a timer for {L('duration',F(DUR))}",
lambda:f"timer {L('duration',F(DUR))}",
lambda:f"alarm {L('date_time',F(DT))}",
lambda:f"open {L('app',F(APP))}",
lambda:f"launch {L('app',F(APP))}",
lambda:f"close {L('app',F(APP))}",
lambda:f"switch to {L('app',F(APP))}",
lambda:f"go to {L('website',F(WEB))}",
lambda:f"open {L('website',F(WEB))}",
lambda:f"search {L('website',F(WEB))} for {L('query',F(QUERY))}",
lambda:f"turn on {L('setting',F(SET))}",
lambda:f"turn off {L('setting',F(SET))}",
lambda:f"enable {L('setting',F(SET))}",
lambda:f"switch {L('setting',F(SET))} off",
lambda:f"set {L('setting',F(SETN))} to {L('number',F(NUM))}",
lambda:f"{F(['increase','lower','raise','reduce'])} the {L('setting',F(SETN))}",
lambda:f"mute {L('setting','volume')}" ,
lambda:f"play {L('media',F(MEDIA))}",
lambda:f"play {L('media',F(MEDIA))} on {L('app','Spotify')}",
lambda:f"put on {L('media',F(MEDIA))}",
lambda:f"find the {L('file',F(FILE))}",
lambda:f"open my {L('file',F(FILE))}",
lambda:f"find {L('file',F(FILE))} in {L('folder',F(FOLD))}",
lambda:f"move {L('file',F(FILE))} to {L('folder',F(FOLD))}",
lambda:f"rename {L('file',F(FILE))} to {L('file',F(FILE))}",
lambda:f"delete the {L('file',F(FILE))}",
lambda:f"open the {L('folder',F(FOLD))} folder",
lambda:f"show me {L('file',F(FILE))} from {L('date_time',F(['yesterday','last week','two weeks ago','last month']))}",
lambda:f"{L('query',F(QUERY))}",
lambda:f"what's the {L('query',F(QUERY))}",
lambda:f"weather in {L('place',F(PLACE))} {L('date_time',F(DT))}",
lambda:f"convert {L('query',F(UNIT))}",
lambda:f"what is {L('query',F(['15% of 2400','sqrt of 144','240 divided by 12','45 plus 78']))}",
lambda:f"add {L('event',F(EVT))} on {L('date_time',F(DT))}",
lambda:f"schedule {L('event',F(EVT))} with {c(F(C))} {L('date_time',F(DT))}",
lambda:f"book {L('event',F(EVT))} {L('date_time',F(DT))}",
lambda:f"call {c(F(C))} and tell {c(F(C))} {L('message',F(MSG))}",
lambda:f"hi",lambda:f"thanks",lambda:f"what can you do",lambda:f"tell me a joke",lambda:f"never mind",
]
def typo(s):
    # lowercase / strip punctuation noise in plain text only (not inside spans handled by brackets)
    r=random.random()
    if r<0.25:s=s.lower() if '[' not in s else re.sub(r'^(\w)',lambda m:m.group(1).lower(),s)
    return s
out=[];seen=set()
n=0
while len(out)<1800:
    s=F(PRE)+F(T)()+F(SUF)
    s=typo(s.strip())
    if s in seen:continue
    seen.add(s);out.append(f"G{len(out)+1:04d}|{s}")
open("data/train_gen.txt","w",encoding="utf-8").write("\n".join(out)+"\n")
print(len(out))
